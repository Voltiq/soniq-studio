//! Time signatures over the song: how many beats each bar has, and where the
//! bar lines fall when that changes along the way.

use serde::{Deserialize, Serialize};

/// From bar `bar` on (0-based; the song's own signature covers bar 0), bars
/// are `beats_per_bar` beats long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterChange {
    pub bar: u32,
    pub beats_per_bar: u32,
}

/// A stretch of bars of one length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeterSegment {
    /// The beat its first bar starts on.
    pub start: f64,
    /// That bar's index.
    pub bar: u32,
    pub beats_per_bar: u32,
}

/// Where the bar lines are.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterMap {
    segments: Vec<MeterSegment>,
}

impl MeterMap {
    /// `base` beats per bar from the start, then each change at its bar
    /// (changes at bar 0, out of order or repeating the length before them
    /// are ignored).
    pub fn new(base: u32, changes: &[MeterChange]) -> Self {
        let mut segments = vec![MeterSegment { start: 0.0, bar: 0, beats_per_bar: base.max(1) }];
        let mut sorted: Vec<MeterChange> = changes.iter().copied().filter(|c| c.bar > 0).collect();
        sorted.sort_by_key(|c| c.bar);
        for change in sorted {
            let last = segments[segments.len() - 1];
            if change.bar <= last.bar || change.beats_per_bar.max(1) == last.beats_per_bar {
                continue;
            }
            let start = last.start + f64::from((change.bar - last.bar) * last.beats_per_bar);
            segments.push(MeterSegment { start, bar: change.bar, beats_per_bar: change.beats_per_bar.max(1) });
        }
        Self { segments }
    }

    pub fn segments(&self) -> &[MeterSegment] {
        &self.segments
    }

    fn segment_at(&self, beat: f64) -> MeterSegment {
        let i = self.segments.partition_point(|s| s.start <= beat + 1e-9);
        self.segments[i.saturating_sub(1)]
    }

    /// The bar `beat` is in, how far into it (beats), and that bar's length.
    pub fn bar_at(&self, beat: f64) -> (u32, f64, u32) {
        let seg = self.segment_at(beat.max(0.0));
        let into = (beat.max(0.0) - seg.start).max(0.0);
        let bars = (into / f64::from(seg.beats_per_bar) + 1e-9).floor();
        (seg.bar + bars as u32, into - bars * f64::from(seg.beats_per_bar), seg.beats_per_bar)
    }

    /// The beat bar `bar` starts on.
    pub fn bar_start(&self, bar: u32) -> f64 {
        let i = self.segments.partition_point(|s| s.bar <= bar);
        let seg = self.segments[i.saturating_sub(1)];
        seg.start + f64::from((bar - seg.bar) * seg.beats_per_bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_lines_follow_the_changes() {
        // Four bars of 4, then two of 3, then 5s.
        let map = MeterMap::new(4, &[MeterChange { bar: 6, beats_per_bar: 5 }, MeterChange { bar: 4, beats_per_bar: 3 }, MeterChange { bar: 0, beats_per_bar: 7 }]);
        assert_eq!(map.segments().iter().map(|s| (s.start, s.bar, s.beats_per_bar)).collect::<Vec<_>>(), [(0.0, 0, 4), (16.0, 4, 3), (22.0, 6, 5)]);
        assert_eq!(map.bar_start(3), 12.0);
        assert_eq!(map.bar_start(5), 19.0);
        assert_eq!(map.bar_start(8), 32.0);
        assert_eq!(map.bar_at(15.5), (3, 3.5, 4));
        assert_eq!(map.bar_at(16.0), (4, 0.0, 3));
        assert_eq!(map.bar_at(21.0), (5, 2.0, 3));
        assert_eq!(map.bar_at(28.0), (7, 1.0, 5));
        // A change to the length already in force is no change.
        assert_eq!(MeterMap::new(4, &[MeterChange { bar: 2, beats_per_bar: 4 }]).segments().len(), 1);
    }
}
