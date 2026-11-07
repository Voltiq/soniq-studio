use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};

/// Transport state shared between the UI thread and the audio callback.
///
/// Everything is atomic so the audio thread never takes a lock. Values are
/// independent, so relaxed ordering is enough.
pub struct Transport {
    playing: AtomicBool,
    metronome: AtomicBool,
    bpm_bits: AtomicU64,
    beats_per_bar: AtomicU32,
    position: AtomicU64,
    sample_rate: AtomicU32,
    looping: AtomicBool,
    loop_start_bits: AtomicU64,
    loop_end_bits: AtomicU64,
    /// Count-in: clicks play but the playhead waits.
    count_in_total: AtomicU64,
    count_in_left: AtomicU64,
}

impl Transport {
    pub fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            metronome: AtomicBool::new(false),
            bpm_bits: AtomicU64::new(120.0_f64.to_bits()),
            beats_per_bar: AtomicU32::new(4),
            position: AtomicU64::new(0),
            sample_rate: AtomicU32::new(48_000),
            looping: AtomicBool::new(false),
            loop_start_bits: AtomicU64::new(0.0_f64.to_bits()),
            loop_end_bits: AtomicU64::new(0.0_f64.to_bits()),
            count_in_total: AtomicU64::new(0),
            count_in_left: AtomicU64::new(0),
        }
    }

    /// Clicks for `samples` before the playhead starts moving.
    pub fn start_count_in(&self, samples: u64) {
        self.count_in_total.store(samples, Relaxed);
        self.count_in_left.store(samples, Relaxed);
    }

    /// (total, remaining) count-in samples.
    pub fn count_in(&self) -> (u64, u64) {
        (self.count_in_total.load(Relaxed), self.count_in_left.load(Relaxed))
    }

    pub fn consume_count_in(&self, samples: u64) {
        let _ = self.count_in_left.try_update(Relaxed, Relaxed, |left| Some(left.saturating_sub(samples)));
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Relaxed)
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Relaxed);
        if !playing {
            self.count_in_left.store(0, Relaxed);
        }
    }

    pub fn metronome_enabled(&self) -> bool {
        self.metronome.load(Relaxed)
    }

    pub fn set_metronome(&self, enabled: bool) {
        self.metronome.store(enabled, Relaxed);
    }

    pub fn bpm(&self) -> f64 {
        f64::from_bits(self.bpm_bits.load(Relaxed))
    }

    pub fn set_bpm(&self, bpm: f64) {
        let bpm = if bpm.is_finite() { bpm.clamp(20.0, 999.0) } else { 120.0 };
        self.bpm_bits.store(bpm.to_bits(), Relaxed);
    }

    pub fn beats_per_bar(&self) -> u32 {
        self.beats_per_bar.load(Relaxed)
    }

    pub fn set_beats_per_bar(&self, beats: u32) {
        self.beats_per_bar.store(beats.clamp(1, 32), Relaxed);
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Relaxed)
    }

    /// Changes the rate while keeping the playhead at the same time in seconds.
    pub fn set_sample_rate(&self, rate: u32) {
        let seconds = self.position_seconds();
        self.sample_rate.store(rate.max(1), Relaxed);
        self.seek_seconds(seconds);
    }

    pub fn position_samples(&self) -> u64 {
        self.position.load(Relaxed)
    }

    pub fn position_seconds(&self) -> f64 {
        self.position_samples() as f64 / f64::from(self.sample_rate())
    }

    pub fn seek_seconds(&self, seconds: f64) {
        let seconds = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        let samples = (seconds * f64::from(self.sample_rate())).round() as u64;
        self.position.store(samples, Relaxed);
    }

    /// Loop bounds in seconds; the renderer wraps sample-accurately.
    pub fn set_loop(&self, enabled: bool, start: f64, end: f64) {
        let start = if start.is_finite() { start.max(0.0) } else { 0.0 };
        let end = if end.is_finite() { end.max(start) } else { start };
        self.loop_start_bits.store(start.to_bits(), Relaxed);
        self.loop_end_bits.store(end.to_bits(), Relaxed);
        self.looping.store(enabled, Relaxed);
    }

    pub fn loop_range(&self) -> (bool, f64, f64) {
        (
            self.looping.load(Relaxed),
            f64::from_bits(self.loop_start_bits.load(Relaxed)),
            f64::from_bits(self.loop_end_bits.load(Relaxed)),
        )
    }

    /// Called by the audio thread at the end of each block. If the exchange
    /// fails the UI seeked while the block was rendering, and the seek wins.
    pub fn advance(&self, from: u64, to: u64) {
        let _ = self.position.compare_exchange(from, to, Relaxed, Relaxed);
    }
}

impl Default for Transport {
    fn default() -> Self {
        Self::new()
    }
}
