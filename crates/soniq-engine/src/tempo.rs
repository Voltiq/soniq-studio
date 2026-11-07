//! Beats ↔ samples on the audio thread: the song's tempo track at the
//! engine rate. Built on the control thread; lookups never allocate.

#[derive(Debug, Clone, Copy, PartialEq)]
struct Segment {
    beat: f64,
    sample: f64,
    bpm: f64,
    /// Tempo change per beat (0: constant).
    slope: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    segments: Vec<Segment>,
    sample_rate: f64,
}

impl TempoMap {
    pub fn constant(bpm: f64, sample_rate: f64) -> Self {
        Self { segments: vec![Segment { beat: 0.0, sample: 0.0, bpm: bpm.max(1.0), slope: 0.0 }], sample_rate }
    }

    /// From stretches of (start beat, start second, bpm, bpm change per beat), in order.
    pub fn new(segments: impl IntoIterator<Item = (f64, f64, f64, f64)>, sample_rate: f64) -> Self {
        let segments: Vec<Segment> =
            segments.into_iter().map(|(beat, seconds, bpm, slope)| Segment { beat, sample: seconds * sample_rate, bpm: bpm.max(1.0), slope }).collect();
        if segments.is_empty() { Self::constant(120.0, sample_rate) } else { Self { segments, sample_rate } }
    }

    /// Follows a single tempo in place (no allocation): the renderer's fallback.
    pub(crate) fn set_constant(&mut self, bpm: f64, sample_rate: f64) {
        self.segments.truncate(1);
        self.segments[0] = Segment { beat: 0.0, sample: 0.0, bpm: bpm.max(1.0), slope: 0.0 };
        self.sample_rate = sample_rate;
    }

    fn by_beat(&self, beat: f64) -> &Segment {
        let i = self.segments.partition_point(|s| s.beat <= beat);
        &self.segments[i.saturating_sub(1)]
    }

    fn by_sample(&self, sample: f64) -> &Segment {
        let i = self.segments.partition_point(|s| s.sample <= sample);
        &self.segments[i.saturating_sub(1)]
    }

    pub fn beat_at(&self, sample: f64) -> f64 {
        let s = self.by_sample(sample);
        let t = (sample - s.sample) / self.sample_rate;
        if s.slope.abs() < 1e-9 { s.beat + t * s.bpm / 60.0 } else { s.beat + s.bpm * ((s.slope * t / 60.0).exp() - 1.0) / s.slope }
    }

    pub fn sample_at(&self, beat: f64) -> f64 {
        let s = self.by_beat(beat);
        let b = beat - s.beat;
        let t = if s.slope.abs() < 1e-9 { b * 60.0 / s.bpm } else { 60.0 / s.slope * ((s.bpm + s.slope * b) / s.bpm).ln() };
        s.sample + t * self.sample_rate
    }

    pub fn samples_per_beat_at(&self, sample: f64) -> f64 {
        let s = self.by_sample(sample);
        let bpm = s.bpm + s.slope * (self.beat_at(sample) - s.beat).max(0.0);
        self.sample_rate * 60.0 / bpm.max(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ramp_is_the_same_both_ways() {
        // 120 BPM for 4 beats, then a ramp to 60 over 4 beats, at 1 kHz.
        let ramp = 60.0 / -15.0 * 0.5f64.ln();
        let map = TempoMap::new([(0.0, 0.0, 120.0, 0.0), (4.0, 2.0, 120.0, -15.0), (8.0, 2.0 + ramp, 60.0, 0.0)], 1000.0);
        assert!((map.sample_at(4.0) - 2000.0).abs() < 1e-6);
        assert!((map.samples_per_beat_at(1000.0) - 500.0).abs() < 1e-6);
        assert!((map.samples_per_beat_at(map.sample_at(10.0)) - 1000.0).abs() < 1e-6);
        for beat in [0.5, 4.0, 5.5, 7.99, 8.0, 12.0] {
            assert!((map.beat_at(map.sample_at(beat)) - beat).abs() < 1e-9, "{beat}");
        }
    }
}
