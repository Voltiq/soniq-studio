//! Eight-band parametric EQ. Each band is a low or high cut (24 dB/oct), a
//! shelf, a bell or a notch, from the RBJ cookbook. The UI draws the same
//! curves (src/state/eqCurve.ts).
//!
//! In linear-phase mode the same curve is applied without shifting any
//! frequency in time: the bands' combined level at every frequency becomes a
//! symmetric filter (a windowed FIR), run by FFT in overlapping frames. It
//! costs a fixed delay, which the engine makes up for on every other path.

use std::f64::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use super::{Smooth, choice};
use crate::Params;
use crate::common::db_gain;

pub const BANDS: usize = 8;
pub const BAND_TYPES: [&str; 6] = ["Low cut", "Low shelf", "Bell", "Notch", "High shelf", "High cut"];

/// Each band's starting type and frequency (mirrors shared/device-catalog.json).
const DEFAULT_TYPE: [f64; BANDS] = [0.0, 1.0, 2.0, 2.0, 2.0, 2.0, 4.0, 5.0];
const DEFAULT_FREQ: [f64; BANDS] = [30.0, 100.0, 250.0, 800.0, 2500.0, 6000.0, 10_000.0, 18_000.0];
const DEFAULT_ON: [f64; BANDS] = [0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0];

#[derive(Debug, Clone, Copy, Default)]
struct Coefs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

/// Transposed direct form II memory for one channel of one stage.
#[derive(Debug, Clone, Copy, Default)]
struct State {
    z1: f64,
    z2: f64,
}

impl State {
    fn tick(&mut self, c: &Coefs, x: f64) -> f64 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// RBJ cookbook biquad, normalised by a0.
fn biquad(kind: usize, freq: f64, gain_db: f64, q: f64, sr: f64) -> Coefs {
    let w = 2.0 * PI * freq.clamp(10.0, sr * 0.49) / sr;
    let (cos, sin) = (w.cos(), w.sin());
    let alpha = sin / (2.0 * q.max(0.05));
    let a = 10f64.powf(gain_db / 40.0);
    let sqa = 2.0 * a.sqrt() * alpha;
    let (b0, b1, b2, a0, a1, a2) = match kind {
        // Low cut (high-pass).
        0 => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        1 => (
            a * ((a + 1.0) - (a - 1.0) * cos + sqa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sqa),
            (a + 1.0) + (a - 1.0) * cos + sqa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sqa,
        ),
        2 => (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a),
        3 => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        4 => (
            a * ((a + 1.0) + (a - 1.0) * cos + sqa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sqa),
            (a + 1.0) - (a - 1.0) * cos + sqa,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sqa,
        ),
        // High cut (low-pass).
        _ => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
    };
    Coefs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

/// The level a biquad passes at `w` radians per sample.
fn magnitude(c: &Coefs, w: f64) -> f64 {
    let (cos1, sin1, cos2, sin2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let num = ((c.b0 + c.b1 * cos1 + c.b2 * cos2).powi(2) + (c.b1 * sin1 + c.b2 * sin2).powi(2)).sqrt();
    let den = ((1.0 + c.a1 * cos1 + c.a2 * cos2).powi(2) + (c.a1 * sin1 + c.a2 * sin2).powi(2)).sqrt();
    num / den.max(1e-12)
}

/// The linear-phase engine: a symmetric filter of `taps` samples, applied by
/// overlap-save FFT every `hop` samples. Everything is allocated when built.
struct Linear {
    taps: usize,
    size: usize,
    hop: usize,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    /// Turns the wanted levels into the filter (`taps` points).
    design_ifft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    /// A Blackman window over the filter, so its ends fade out.
    window: Vec<f32>,
    design: Vec<Complex32>,
    /// The filter's spectrum (`size` points).
    kernel: Vec<Complex32>,
    /// The last `size` input samples, left and right; the newest `hop` are being filled.
    input: [Vec<f32>; 2],
    filled: usize,
    /// The filtered block being played out.
    output: [Vec<f32>; 2],
    work: Vec<Complex32>,
    /// The bands changed: the filter is made again at the next block.
    dirty: bool,
}

impl Linear {
    fn new(sr: f64) -> Self {
        // About 0.17 s of filter: fine enough for steep cuts down to 30 Hz.
        let taps = if sr <= 50_000.0 {
            8192
        } else if sr <= 100_000.0 {
            16_384
        } else {
            32_768
        };
        let (size, hop) = (2 * taps, taps / 8);
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(size);
        let ifft = planner.plan_fft_inverse(size);
        let design_ifft = planner.plan_fft_inverse(taps);
        let scratch = fft.get_inplace_scratch_len().max(ifft.get_inplace_scratch_len()).max(design_ifft.get_inplace_scratch_len());
        let window = (0..taps)
            .map(|n| {
                let x = 2.0 * PI * n as f64 / taps as f64;
                (0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos()) as f32
            })
            .collect();
        Self {
            taps,
            size,
            hop,
            fft,
            ifft,
            design_ifft,
            scratch: vec![Complex32::default(); scratch],
            window,
            design: vec![Complex32::default(); taps],
            kernel: vec![Complex32::default(); size],
            input: [vec![0.0; size], vec![0.0; size]],
            filled: 0,
            output: [vec![0.0; hop], vec![0.0; hop]],
            work: vec![Complex32::default(); size],
            dirty: true,
        }
    }

    /// Samples the output comes late by: one block, and half the filter.
    fn latency(&self) -> u32 {
        (self.hop + self.taps / 2) as u32
    }

    /// Makes the filter from the bands' combined level at every frequency.
    fn design(&mut self, bands: &[Band; BANDS]) {
        let taps = self.taps;
        for k in 0..=taps / 2 {
            let w = 2.0 * PI * k as f64 / taps as f64;
            let mut level = 1.0;
            for band in bands.iter().filter(|b| b.on) {
                for c in &band.coefs[..band.stages] {
                    level *= magnitude(c, w);
                }
            }
            // No phase: a real, symmetric spectrum.
            let v = Complex32::new(level.min(1e4) as f32, 0.0);
            self.design[k] = v;
            self.design[(taps - k) % taps] = v;
        }
        self.design_ifft.process_with_scratch(&mut self.design, &mut self.scratch);
        // Centred in the filter, windowed, and transformed at the frame size.
        let norm = 1.0 / taps as f32;
        for (n, k) in self.kernel.iter_mut().enumerate() {
            *k = if n < taps { Complex32::new(self.design[(n + taps / 2) % taps].re * norm * self.window[n], 0.0) } else { Complex32::default() };
        }
        self.fft.process_with_scratch(&mut self.kernel, &mut self.scratch);
        self.dirty = false;
    }

    /// One sample in, one (late) sample out, per channel.
    fn tick(&mut self, bands: &[Band; BANDS], l: f32, r: f32) -> (f32, f32) {
        let at = self.size - self.hop + self.filled;
        self.input[0][at] = l;
        self.input[1][at] = r;
        let out = (self.output[0][self.filled], self.output[1][self.filled]);
        self.filled += 1;
        if self.filled == self.hop {
            self.filled = 0;
            if self.dirty {
                self.design(bands);
            }
            let (size, hop) = (self.size, self.hop);
            let norm = 1.0 / size as f32;
            for ch in 0..2 {
                for (w, x) in self.work.iter_mut().zip(&self.input[ch]) {
                    *w = Complex32::new(*x, 0.0);
                }
                self.fft.process_with_scratch(&mut self.work, &mut self.scratch);
                for (w, k) in self.work.iter_mut().zip(&self.kernel) {
                    *w *= k;
                }
                self.ifft.process_with_scratch(&mut self.work, &mut self.scratch);
                // The end of the frame is free of wrap-around: the newest block, filtered.
                for (o, w) in self.output[ch].iter_mut().zip(&self.work[size - hop..]) {
                    *o = w.re * norm;
                }
                self.input[ch].copy_within(hop.., 0);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Band {
    on: bool,
    /// Cuts run two stages (24 dB/oct): with Q 0.71 they are a 4th-order Butterworth.
    stages: usize,
    coefs: [Coefs; 2],
    state: [[State; 2]; 2],
}

/// Each band's parameter ids: on, type, frequency, gain, Q (no formatting on the audio thread).
const IDS: [[&str; 5]; BANDS] = [
    ["b1on", "b1type", "b1freq", "b1gain", "b1q"],
    ["b2on", "b2type", "b2freq", "b2gain", "b2q"],
    ["b3on", "b3type", "b3freq", "b3gain", "b3q"],
    ["b4on", "b4type", "b4freq", "b4gain", "b4q"],
    ["b5on", "b5type", "b5freq", "b5gain", "b5q"],
    ["b6on", "b6type", "b6freq", "b6gain", "b6q"],
    ["b7on", "b7type", "b7freq", "b7gain", "b7q"],
    ["b8on", "b8type", "b8freq", "b8gain", "b8q"],
];

pub struct Eq {
    sr: f64,
    bands: [Band; BANDS],
    output: Smooth,
    /// Linear-phase mode, chosen when the effect is built (its delay is part of the graph).
    linear: Option<Box<Linear>>,
}

impl Eq {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let linear = (choice(p, "phase", 0.0) == 1).then(|| Box::new(Linear::new(f64::from(sr))));
        let mut eq = Self { sr: f64::from(sr), bands: [Band::default(); BANDS], output: Smooth::new(db_gain(p.or("output", 0.0)), sr), linear };
        eq.configure(p);
        eq
    }

    /// Samples the output comes late by (none in minimum-phase mode).
    pub fn latency(&self) -> u32 {
        self.linear.as_ref().map_or(0, |l| l.latency())
    }

    pub fn configure(&mut self, p: &dyn Params) {
        for (i, band) in self.bands.iter_mut().enumerate() {
            let [on, kind, freq, gain, q] = IDS[i];
            let kind = choice(p, kind, DEFAULT_TYPE[i]).min(5);
            let freq = p.or(freq, DEFAULT_FREQ[i]);
            let gain = p.or(gain, 0.0);
            let q = p.or(q, 0.71);
            // A bell or shelf at 0 dB does nothing: skip it.
            band.on = p.or(on, DEFAULT_ON[i]) >= 0.5 && (matches!(kind, 0 | 3 | 5) || gain.abs() > 0.01);
            if matches!(kind, 0 | 5) {
                // Butterworth Q pair scaled by the band's Q (0.71 gives the flat response).
                band.coefs = [biquad(kind, freq, 0.0, q * 1.307 / 0.707, self.sr), biquad(kind, freq, 0.0, 0.541, self.sr)];
                band.stages = 2;
            } else {
                band.coefs[0] = biquad(kind, freq, gain, q, self.sr);
                band.stages = 1;
            }
        }
        self.output.set(db_gain(p.or("output", 0.0)));
        if let Some(linear) = &mut self.linear {
            linear.dirty = true;
        }
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if let Some(linear) = &mut self.linear {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let (a, b) = linear.tick(&self.bands, *l, *r);
                let g = self.output.next();
                *l = a * g;
                *r = b * g;
            }
            return;
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (mut a, mut b) = (f64::from(*l), f64::from(*r));
            for band in self.bands.iter_mut().filter(|b| b.on) {
                for s in 0..band.stages {
                    a = band.state[s][0].tick(&band.coefs[s], a);
                    b = band.state[s][1].tick(&band.coefs[s], b);
                }
            }
            let g = self.output.next();
            *l = a as f32 * g;
            *r = b as f32 * g;
        }
    }

    pub fn adopt(&mut self, old: &mut Eq) {
        for (new, old) in self.bands.iter_mut().zip(&old.bands) {
            new.state = old.state;
        }
        self.output.adopt(&old.output);
        // The sound still inside the linear filter carries on; its curve follows the new bands.
        if let (Some(new), Some(old)) = (&mut self.linear, &mut old.linear)
            && new.size == old.size
        {
            std::mem::swap(new, old);
            new.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;

    fn gain_at(params: &[(&str, f64)], hz: f32) -> f32 {
        let mut eq = AudioFx::build("eq", SR, &params_of(params)).unwrap();
        let input = sine(hz, 0.1, 48_000);
        let (l, _) = run(&mut eq, &input);
        db(rms(&l[24_000..])) - db(rms(&input[24_000..]))
    }

    fn params_of<'a>(list: &'a [(&'a str, f64)]) -> impl Fn(&str) -> Option<f64> + 'a {
        params(list)
    }

    #[test]
    fn a_bell_boosts_its_frequency_and_leaves_the_rest() {
        let bell = [("b4freq", 1000.0), ("b4gain", 12.0), ("b4q", 1.0)];
        assert!((gain_at(&bell, 1000.0) - 12.0).abs() < 0.3, "{}", gain_at(&bell, 1000.0));
        assert!(gain_at(&bell, 100.0).abs() < 0.5);
        // Untouched, the EQ is transparent.
        assert!(gain_at(&[], 440.0).abs() < 0.01);
    }

    #[test]
    fn a_low_cut_removes_rumble_at_24_db_per_octave() {
        let cut = [("b1on", 1.0), ("b1freq", 200.0)];
        assert!(gain_at(&cut, 50.0) < -40.0, "{}", gain_at(&cut, 50.0));
        assert!((gain_at(&cut, 200.0) + 3.0).abs() < 0.5, "−3 dB at the corner: {}", gain_at(&cut, 200.0));
        assert!(gain_at(&cut, 2000.0).abs() < 0.1);
    }

    #[test]
    fn linear_phase_has_the_same_curve_and_a_symmetric_response() {
        let bell = [("phase", 1.0), ("b4freq", 1000.0), ("b4gain", 12.0), ("b4q", 1.0)];
        assert!((gain_at(&bell, 1000.0) - 12.0).abs() < 0.3, "{}", gain_at(&bell, 1000.0));
        assert!(gain_at(&bell, 100.0).abs() < 0.3);
        let cut = [("phase", 1.0), ("b1on", 1.0), ("b1freq", 200.0)];
        assert!(gain_at(&cut, 50.0) < -40.0, "{}", gain_at(&cut, 50.0));
        assert!((gain_at(&cut, 200.0) + 3.0).abs() < 0.5, "{}", gain_at(&cut, 200.0));
        // An impulse comes out centred on the reported delay, the same before as after it.
        let mut eq = AudioFx::build("eq", SR, &params_of(&bell)).unwrap();
        let late = eq.latency() as usize;
        assert_eq!(late, 1024 + 4096);
        let mut input = vec![0.0_f32; 12_000];
        input[0] = 1.0;
        let (out, _) = run(&mut eq, &input);
        let peak_at = (0..out.len()).max_by(|a, b| out[*a].abs().total_cmp(&out[*b].abs())).unwrap();
        assert_eq!(peak_at, late);
        assert!((1..2000).all(|k| (out[late - k] - out[late + k]).abs() < 1e-5), "not symmetric");
        // Minimum phase reports no delay.
        assert_eq!(AudioFx::build("eq", SR, &params_of(&[])).unwrap().latency(), 0);
    }

    #[test]
    fn a_linear_eq_follows_its_knobs_and_keeps_its_sound_across_a_rebuild() {
        let mut eq = AudioFx::build("eq", SR, &params_of(&[("phase", 1.0)])).unwrap();
        let input = sine(1000.0, 0.1, 48_000);
        let (flat, _) = run(&mut eq, &input);
        assert!((db(rms(&flat[24_000..])) - db(rms(&input[24_000..]))).abs() < 0.05, "flat is transparent");
        // A rebuilt EQ (a knob turned) takes over the sound in flight: no gap, the new curve.
        let mut next = AudioFx::build("eq", SR, &params_of(&[("phase", 1.0), ("b4freq", 1000.0), ("b4gain", -12.0), ("b4q", 1.0)])).unwrap();
        next.adopt(&mut eq);
        let (cut, _) = run(&mut next, &input);
        // (A fresh one would be silent for its whole delay; this one is already 12 dB down.)
        assert!(rms(&cut[..2000]) > 0.015, "the sound in flight plays on: {}", rms(&cut[..2000]));
        assert!((db(rms(&cut[24_000..])) - db(rms(&input[24_000..])) + 12.0).abs() < 0.3);
    }

    #[test]
    fn shelves_and_notches() {
        let shelf = [("b7freq", 4000.0), ("b7gain", -6.0)];
        assert!((gain_at(&shelf, 16_000.0) + 6.0).abs() < 0.5);
        assert!(gain_at(&shelf, 200.0).abs() < 0.3);
        let notch = [("b4type", 3.0), ("b4freq", 1000.0), ("b4q", 4.0)];
        assert!(gain_at(&notch, 1000.0) < -30.0);
    }
}
