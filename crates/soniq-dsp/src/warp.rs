//! Warping: audio that follows the song's tempo. The engine says which
//! source sample each output sample should play (a [`WarpClock`]); a
//! [`Warper`] turns that into sound in one of three ways:
//!
//! - **Repitch** reads the source faster or slower, like a record: the pitch follows.
//! - **Beats** plays each hit (the stretch from one transient to the next)
//!   at its own speed, started where it falls on the song's beat. Slower than
//!   the original, a hit fades out before the next one would sound; faster,
//!   the next one cuts it short. Drums keep their punch.
//! - **Tones** overlap-adds short grains read at the warped position, each
//!   nudged to where the waveform best continues the one before (WSOLA).
//!   The pitch stays put, for melodies, chords and voices.
//!
//! Nothing here allocates once built.

use std::f64::consts::PI;
use std::sync::Arc;

use soniq_io::AudioBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpMode {
    Beats,
    Tones,
    Repitch,
}

/// Where warped audio comes from: the source sample (in the buffer) each
/// output sample plays, and the other way round.
pub trait WarpClock {
    fn source_at(&self, out: f64) -> f64;
    fn output_at(&self, source: f64) -> f64;
}

/// Grains remembered between blocks (a block needs at most three).
const CACHE: usize = 6;

pub struct Warper {
    mode: WarpMode,
    /// Beats: where hits start, in source samples, sorted.
    onsets: Arc<Vec<usize>>,
    /// Beats: the crossfade into a hit and the fade out at its end, in samples.
    xfade: f64,
    fade: f64,
    /// Tones: grain length and hop (half of it), the window, and how far a
    /// grain may move to line up.
    grain: usize,
    hop: usize,
    window: Vec<f32>,
    tolerance: i64,
    /// Tones: (grain index, source start) of recent grains.
    grains: [(i64, i64); CACHE],
    next: usize,
}

impl Warper {
    /// Built on the control thread. `onsets` are needed for Beats only.
    pub fn new(mode: WarpMode, onsets: Option<Arc<Vec<usize>>>, sample_rate: u32) -> Self {
        let sr = f64::from(sample_rate.max(1));
        let grain = (((0.043 * sr) as usize) / 2 * 2).max(64);
        let window = if mode == WarpMode::Tones {
            // Periodic Hann: two at 50 % overlap sum to exactly one.
            (0..grain).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / grain as f64).cos()) as f32).collect()
        } else {
            Vec::new()
        };
        Self {
            mode,
            onsets: onsets.unwrap_or_default(),
            xfade: 0.002 * sr,
            fade: 0.012 * sr,
            grain,
            hop: grain / 2,
            window,
            tolerance: (0.012 * sr) as i64,
            grains: [(i64::MIN, 0); CACHE],
            next: 0,
        }
    }

    /// Keeps the grains the old one had lined up, so an edit elsewhere never
    /// makes a stretched clip stumble.
    pub fn adopt(&mut self, old: &Warper) {
        if self.mode == old.mode && self.grain == old.grain {
            self.grains = old.grains;
            self.next = old.next;
        }
    }

    /// Writes (not adds) the warped audio for output samples `out_start…` into `left` and `right`.
    pub fn render<C: WarpClock>(&mut self, buffer: &AudioBuffer, clock: &C, out_start: u64, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let Some(l) = buffer.channels.first() else {
            left[..n].fill(0.0);
            right[..n].fill(0.0);
            return;
        };
        let r = buffer.channels.get(1).unwrap_or(l);
        match self.mode {
            WarpMode::Repitch => {
                for i in 0..n {
                    let src = clock.source_at((out_start + i as u64) as f64);
                    left[i] = cubic(l, src);
                    right[i] = cubic(r, src);
                }
            }
            WarpMode::Beats => {
                for i in 0..n {
                    let (a, b) = self.beats_sample(l, r, clock, (out_start + i as u64) as f64);
                    left[i] = a;
                    right[i] = b;
                }
            }
            WarpMode::Tones => self.tones(l, r, clock, out_start, &mut left[..n], &mut right[..n]),
        }
    }

    /// Hit `k` (or the stretch before the first hit, `k == None`): its start and length in the source.
    fn hit(&self, k: Option<usize>) -> (f64, f64) {
        let start = k.map_or(0.0, |k| self.onsets[k] as f64);
        let next = match k {
            None => self.onsets.first().map(|&o| o as f64),
            Some(k) => self.onsets.get(k + 1).map(|&o| o as f64),
        };
        (start, next.map_or(f64::INFINITY, |n| n - start))
    }

    /// One sample of hit `k`, `d` output samples after it started (`None` before the first hit).
    fn hit_sample(&self, l: &[f32], r: &[f32], k: Option<usize>, d: f64) -> (f32, f32) {
        let (start, len) = self.hit(k);
        // Fades out before the next hit's own sound would come through.
        let fade = self.fade.min(len / 4.0).max(1.0);
        let g = ((len - d) / fade).clamp(0.0, 1.0) as f32;
        if g <= 0.0 || d < 0.0 {
            return (0.0, 0.0);
        }
        (linear(l, start + d) * g, linear(r, start + d) * g)
    }

    fn beats_sample<C: WarpClock>(&self, l: &[f32], r: &[f32], clock: &C, out: f64) -> (f32, f32) {
        let q = clock.source_at(out);
        let after = self.onsets.partition_point(|&o| (o as f64) <= q);
        let k = after.checked_sub(1);
        let seg_out = clock.output_at(self.hit(k).0);
        let d = out - seg_out;
        let (a, b) = self.hit_sample(l, r, k, d);
        if d >= self.xfade || k.is_none() {
            return (a, b);
        }
        // The first moments of a hit: the one before rings out under it.
        let prev = k.and_then(|k| k.checked_sub(1));
        let prev_d = out - clock.output_at(self.hit(prev).0);
        let (pa, pb) = self.hit_sample(l, r, prev, prev_d);
        let t = (d / self.xfade).clamp(0.0, 1.0) as f32;
        (a * t + pa * (1.0 - t), b * t + pb * (1.0 - t))
    }

    /// The source start of grain `g`, lined up with grain `g − 1` when it is known.
    fn grain_start<C: WarpClock>(&mut self, mono: impl Fn(i64) -> f32, clock: &C, g: i64) -> i64 {
        let nominal = clock.source_at((g * self.hop as i64) as f64).round() as i64;
        if let Some(&(_, p)) = self.grains.iter().find(|(k, _)| *k == g) {
            // A stale position (the tempo changed under it) starts afresh.
            if (p - nominal).abs() <= 2 * self.tolerance {
                return p;
            }
        }
        let chosen = match self.grains.iter().find(|(k, _)| *k == g - 1) {
            Some(&(_, prev)) if (prev + self.hop as i64 - nominal).abs() <= 4 * self.tolerance => {
                // Where the previous grain would have gone on.
                let natural = prev + self.hop as i64;
                let overlap = self.hop as i64;
                let score = |cand: i64, step: usize| -> f32 { (0..overlap).step_by(step).map(|i| mono(natural + i) * mono(cand + i)).sum() };
                // The best match; ties (silence) keep the nominal place.
                let best = |from: i64, candidates: &mut dyn Iterator<Item = i64>, step: usize| {
                    let mut best = (from, score(from, step));
                    for c in candidates {
                        let s = score(c, step);
                        if s > best.1 + 1e-6 {
                            best = (c, s);
                        }
                    }
                    best.0
                };
                let coarse = best(nominal, &mut (-self.tolerance..=self.tolerance).step_by(4).map(|d| nominal + d), 4);
                best(coarse, &mut (coarse - 3..=coarse + 3), 2)
            }
            _ => nominal,
        };
        self.grains[self.next] = (g, chosen);
        self.next = (self.next + 1) % CACHE;
        chosen
    }

    fn tones<C: WarpClock>(&mut self, l: &[f32], r: &[f32], clock: &C, out_start: u64, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        left.fill(0.0);
        right.fill(0.0);
        let hop = self.hop as i64;
        let at = |x: &[f32], i: i64| if i >= 0 && (i as usize) < x.len() { x[i as usize] } else { 0.0 };
        let (first, last) = (out_start as i64 / hop - 1, (out_start as i64 + n as i64 - 1) / hop);
        for g in first..=last {
            let p = self.grain_start(|i| 0.5 * (at(l, i) + at(r, i)), clock, g);
            let grain_out = g * hop;
            // The part of this grain inside the block.
            let from = (grain_out - out_start as i64).max(0);
            let to = (grain_out + self.grain as i64 - out_start as i64).min(n as i64);
            for o in from..to {
                let w_at = (out_start as i64 + o - grain_out) as usize;
                let w = self.window[w_at];
                let src = p + w_at as i64;
                left[o as usize] += at(l, src) * w;
                right[o as usize] += at(r, src) * w;
            }
        }
    }
}

fn linear(x: &[f32], pos: f64) -> f32 {
    if pos < 0.0 {
        return 0.0;
    }
    let i = pos.floor() as usize;
    let frac = (pos - i as f64) as f32;
    let a = x.get(i).copied().unwrap_or(0.0);
    let b = x.get(i + 1).copied().unwrap_or(0.0);
    a + (b - a) * frac
}

/// Catmull-Rom interpolation: smooth enough for repitching.
fn cubic(x: &[f32], pos: f64) -> f32 {
    if pos < -1.0 {
        return 0.0;
    }
    let i = pos.floor() as i64;
    let t = (pos - i as f64) as f32;
    let at = |k: i64| if k >= 0 && (k as usize) < x.len() { x[k as usize] } else { 0.0 };
    let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays the source `ratio` times as fast (constant tempo).
    struct Rate(f64);

    impl WarpClock for Rate {
        fn source_at(&self, out: f64) -> f64 {
            out * self.0
        }
        fn output_at(&self, source: f64) -> f64 {
            source / self.0
        }
    }

    const SR: u32 = 48_000;

    fn tone(hz: f32, frames: usize) -> AudioBuffer {
        AudioBuffer::new(SR, vec![(0..frames).map(|i| (i as f32 * hz * std::f32::consts::TAU / SR as f32).sin() * 0.5).collect()])
    }

    fn render(warper: &mut Warper, buffer: &AudioBuffer, clock: &impl WarpClock, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames];
        let mut right = vec![0.0; frames];
        for (k, (l, r)) in out.chunks_mut(256).zip(right.chunks_mut(256)).enumerate() {
            warper.render(buffer, clock, (k * 256) as u64, l, r);
        }
        out
    }

    /// Upward zero crossings per second.
    fn hz(x: &[f32]) -> f32 {
        let ups: Vec<usize> = (1..x.len()).filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0).collect();
        (ups.len() - 1) as f32 * SR as f32 / (ups[ups.len() - 1] - ups[0]) as f32
    }

    #[test]
    fn repitch_follows_the_tempo_and_tones_keep_the_pitch() {
        let source = tone(440.0, SR as usize * 3);
        // The song is 25 % faster than the audio's own tempo.
        let clock = Rate(1.25);
        let mut repitch = Warper::new(WarpMode::Repitch, None, SR);
        let out = render(&mut repitch, &source, &clock, SR as usize);
        assert!((hz(&out[4800..]) - 550.0).abs() < 2.0, "{}", hz(&out[4800..]));
        let mut tones = Warper::new(WarpMode::Tones, None, SR);
        let out = render(&mut tones, &source, &clock, SR as usize * 2);
        assert!((hz(&out[9600..90_000]) - 440.0).abs() < 3.0, "{}", hz(&out[9600..90_000]));
        // Grains add up to a steady level, without comb-filter dips.
        let peak = out[20_000..80_000].chunks(2400).map(|c| c.iter().fold(0.0_f32, |m, s| m.max(s.abs()))).fold(1.0_f32, f32::min);
        assert!(peak > 0.45, "{peak}");
    }

    #[test]
    fn beats_mode_moves_each_hit_to_its_place() {
        // Four clicks, 12 000 samples apart; at 1.5 × the tempo they come every 8 000.
        let mut data = vec![0.0f32; 48_000];
        for k in 0..4 {
            for i in 0..2000 {
                data[k * 12_000 + i] = (-(i as f32) / 300.0).exp() * if i % 2 == 0 { 1.0 } else { -1.0 };
            }
        }
        let source = AudioBuffer::new(SR, vec![data]);
        let onsets = Arc::new(vec![0, 12_000, 24_000, 36_000]);
        let mut beats = Warper::new(WarpMode::Beats, Some(onsets), SR);
        let out = render(&mut beats, &source, &Rate(1.5), 32_000);
        for k in 0..4 {
            let at = k * 8000;
            // Each hit starts on its place, at full level and at its own speed (no smearing).
            assert!(out[at + 200].abs() > 0.3, "hit {k}: {}", out[at + 200]);
            assert!(out[at + 7000].abs() < 0.01, "between hits {k}: {}", out[at + 7000]);
        }
    }
}
