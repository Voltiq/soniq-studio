//! The tempo track: where beats fall in time when the tempo changes.

use serde::{Deserialize, Serialize};

pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 999.0;

/// A point of the tempo track. Between two points the tempo glides in a
/// straight line (per beat); two points at the same beat make a jump.
/// Before the first point and after the last, the tempo holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TempoPoint {
    /// Beats from the song's start.
    pub time: f64,
    pub bpm: f64,
}

/// A stretch of constant or steadily changing tempo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoSegment {
    /// Where it starts, in beats and in seconds.
    pub beat: f64,
    pub seconds: f64,
    /// The tempo there.
    pub bpm: f64,
    /// How much the tempo changes per beat (0: constant).
    pub slope: f64,
}

impl TempoSegment {
    /// Seconds from the segment's start to `beats` beats into it.
    fn seconds_for(&self, beats: f64) -> f64 {
        if self.slope.abs() < 1e-9 {
            beats * 60.0 / self.bpm
        } else {
            60.0 / self.slope * ((self.bpm + self.slope * beats) / self.bpm).ln()
        }
    }

    /// Beats from the segment's start to `seconds` seconds into it.
    fn beats_for(&self, seconds: f64) -> f64 {
        if self.slope.abs() < 1e-9 {
            seconds * self.bpm / 60.0
        } else {
            self.bpm * ((self.slope * seconds / 60.0).exp() - 1.0) / self.slope
        }
    }
}

/// Beats ↔ seconds for a song, its tempo track included.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    segments: Vec<TempoSegment>,
}

impl TempoMap {
    pub fn constant(bpm: f64) -> Self {
        Self { segments: vec![TempoSegment { beat: 0.0, seconds: 0.0, bpm: bpm.clamp(MIN_BPM, MAX_BPM), slope: 0.0 }] }
    }

    /// The song's tempo `bpm` when the tempo track is empty; the track's points otherwise.
    pub fn new(bpm: f64, points: &[TempoPoint]) -> Self {
        let mut points: Vec<TempoPoint> = points
            .iter()
            .filter(|p| p.time.is_finite() && p.bpm.is_finite())
            .map(|p| TempoPoint { time: p.time.max(0.0), bpm: p.bpm.clamp(MIN_BPM, MAX_BPM) })
            .collect();
        points.sort_by(|a, b| a.time.total_cmp(&b.time));
        let Some(first) = points.first() else { return Self::constant(bpm) };
        // (beat, bpm, slope) of each stretch; seconds are added up after.
        let mut parts = vec![(0.0, first.bpm, 0.0)];
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if b.time - a.time > 1e-9 {
                parts.push((a.time, a.bpm, (b.bpm - a.bpm) / (b.time - a.time)));
            }
        }
        let last = points[points.len() - 1];
        parts.push((last.time, last.bpm, 0.0));
        let mut segments: Vec<TempoSegment> = Vec::with_capacity(parts.len());
        for (beat, bpm, slope) in parts {
            let seconds = segments.last().map_or(0.0, |s| s.seconds + s.seconds_for(beat - s.beat));
            // A segment of no length (a point at beat 0) gives way to the next.
            if segments.last().is_some_and(|s| beat - s.beat < 1e-9) {
                segments.pop();
            }
            segments.push(TempoSegment { beat, seconds, bpm, slope });
        }
        Self { segments }
    }

    pub fn segments(&self) -> &[TempoSegment] {
        &self.segments
    }

    fn by_beat(&self, beats: f64) -> &TempoSegment {
        let i = self.segments.partition_point(|s| s.beat <= beats);
        &self.segments[i.saturating_sub(1)]
    }

    fn by_seconds(&self, seconds: f64) -> &TempoSegment {
        let i = self.segments.partition_point(|s| s.seconds <= seconds);
        &self.segments[i.saturating_sub(1)]
    }

    pub fn seconds_at(&self, beats: f64) -> f64 {
        let s = self.by_beat(beats);
        s.seconds + s.seconds_for(beats - s.beat)
    }

    pub fn beats_at(&self, seconds: f64) -> f64 {
        let s = self.by_seconds(seconds);
        s.beat + s.beats_for(seconds - s.seconds)
    }

    pub fn bpm_at(&self, beats: f64) -> f64 {
        let s = self.by_beat(beats);
        (s.bpm + s.slope * (beats - s.beat).max(0.0)).clamp(MIN_BPM, MAX_BPM)
    }

    /// The tempo never changes.
    pub fn is_constant(&self) -> bool {
        self.segments.len() == 1 && self.segments[0].slope == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn an_empty_track_keeps_the_song_tempo() {
        let map = TempoMap::new(120.0, &[]);
        assert!(close(map.seconds_at(8.0), 4.0) && close(map.beats_at(4.0), 8.0) && map.is_constant());
    }

    #[test]
    fn jumps_and_ramps_move_the_beats() {
        // 120 BPM for 8 beats, a jump to 60, a ramp to 120 over 8 beats, then 120.
        let points = [
            TempoPoint { time: 0.0, bpm: 120.0 },
            TempoPoint { time: 8.0, bpm: 120.0 },
            TempoPoint { time: 8.0, bpm: 60.0 },
            TempoPoint { time: 16.0, bpm: 120.0 },
        ];
        let map = TempoMap::new(100.0, &points);
        assert!(close(map.seconds_at(8.0), 4.0));
        assert!(close(map.bpm_at(8.0), 60.0) && close(map.bpm_at(12.0), 90.0) && close(map.bpm_at(40.0), 120.0));
        // A ramp from 60 to 120 over 8 beats: 60 / 7.5 · ln 2 seconds.
        let ramp = 60.0 / 7.5 * 2f64.ln();
        assert!(close(map.seconds_at(16.0), 4.0 + ramp), "{}", map.seconds_at(16.0));
        assert!(close(map.seconds_at(20.0), 4.0 + ramp + 2.0));
        for beats in [0.0, 3.5, 8.0, 9.25, 15.9, 30.0] {
            assert!(close(map.beats_at(map.seconds_at(beats)), beats), "{beats}");
        }
    }
}
