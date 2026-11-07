//! Tone and utility: a saturator (soft, hard or tube curves, a tilt tone
//! control) and a utility (gain, stereo width, pan, mono, phase invert).

use std::f32::consts::TAU;

use super::{Smooth, choice};
use crate::Params;
use crate::common::db_gain;

/// Where the tilt tone control pivots, in Hz.
const TILT_HZ: f32 = 800.0;
/// The tube curve's bias, which makes it asymmetric (even harmonics).
const BIAS: f32 = 0.3;

pub struct Saturator {
    kind: usize,
    drive: f32,
    /// Keeps the level roughly where it was as the drive goes up.
    compensate: f32,
    tilt: [f32; 2],
    lowpass: f32,
    low: [f32; 2],
    /// DC blocker memory: the tube curve's asymmetry adds an offset.
    dc: [(f32, f32); 2],
    dc_coef: f32,
    mix: f32,
    output: Smooth,
}

impl Saturator {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut sat = Self {
            kind: 0,
            drive: 1.0,
            compensate: 1.0,
            tilt: [1.0; 2],
            lowpass: 1.0 - (-TAU * TILT_HZ / sr).exp(),
            low: [0.0; 2],
            dc: [(0.0, 0.0); 2],
            dc_coef: (-TAU * 10.0 / sr).exp(),
            mix: 1.0,
            output: Smooth::new(db_gain(p.or("output", 0.0)), sr),
        };
        sat.configure(p);
        sat
    }

    pub fn configure(&mut self, p: &dyn Params) {
        let tone = (p.or("tone", 0.0) / 100.0).clamp(-1.0, 1.0) as f32;
        self.kind = choice(p, "type", 0.0).min(2);
        self.drive = db_gain(p.or("drive", 6.0).max(0.0));
        self.compensate = 1.0 / self.drive.sqrt();
        self.tilt = [(-tone).exp2(), tone.exp2()];
        self.mix = (p.or("mix", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
        self.output.set(db_gain(p.or("output", 0.0)));
    }

    fn shape(&self, x: f32) -> f32 {
        match self.kind {
            0 => x.tanh(),
            1 => x.clamp(-1.0, 1.0),
            _ => (x + BIAS).tanh() - BIAS.tanh(),
        }
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let out = self.output.next();
            for (ch, x) in [&mut *l, &mut *r].into_iter().enumerate() {
                let mut y = self.shape(*x * self.drive) * self.compensate;
                if self.kind == 2 {
                    let (x1, y1) = self.dc[ch];
                    let blocked = y - x1 + self.dc_coef * y1;
                    self.dc[ch] = (y, blocked);
                    y = blocked;
                }
                self.low[ch] += (y - self.low[ch]) * self.lowpass;
                let low = self.low[ch];
                y = low * self.tilt[0] + (y - low) * self.tilt[1];
                *x = (*x * (1.0 - self.mix) + y * self.mix) * out;
            }
        }
    }

    pub fn adopt(&mut self, old: &mut Saturator) {
        self.low = old.low;
        self.dc = old.dc;
        self.output.adopt(&old.output);
    }
}

pub struct Utility {
    gain: Smooth,
    width: f32,
    pan: [Smooth; 2],
    invert: [f32; 2],
}

/// Balance: turning one way only turns the other side down.
fn balance(p: &dyn Params) -> [f32; 2] {
    let pan = (p.or("pan", 0.0) / 50.0).clamp(-1.0, 1.0) as f32;
    [1.0 - pan.max(0.0), 1.0 + pan.min(0.0)]
}

impl Utility {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let [l, r] = balance(p);
        let mut utility = Self { gain: Smooth::new(db_gain(p.or("gain", 0.0)), sr), width: 1.0, pan: [Smooth::new(l, sr), Smooth::new(r, sr)], invert: [1.0; 2] };
        utility.configure(p);
        utility
    }

    pub fn configure(&mut self, p: &dyn Params) {
        let mono = choice(p, "mono", 0.0) == 1;
        let invert = choice(p, "invert", 0.0);
        let [l, r] = balance(p);
        self.gain.set(db_gain(p.or("gain", 0.0)));
        self.width = if mono { 0.0 } else { (p.or("width", 100.0) / 100.0).clamp(0.0, 2.0) as f32 };
        self.pan[0].set(l);
        self.pan[1].set(r);
        self.invert = [if invert & 1 == 1 { -1.0 } else { 1.0 }, if invert >= 2 { -1.0 } else { 1.0 }];
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (a, b) = (*l * self.invert[0], *r * self.invert[1]);
            let mid = (a + b) * 0.5;
            let side = (a - b) * 0.5 * self.width;
            let g = self.gain.next();
            *l = (mid + side) * self.pan[0].next() * g;
            *r = (mid - side) * self.pan[1].next() * g;
        }
    }

    pub fn adopt(&mut self, old: &mut Utility) {
        self.gain.adopt(&old.gain);
        self.pan[0].adopt(&old.pan[0]);
        self.pan[1].adopt(&old.pan[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;
    use crate::FxContext;

    #[test]
    fn saturation_squares_off_the_wave() {
        for kind in [0.0, 1.0, 2.0] {
            let mut sat = AudioFx::build("saturator", SR, &params(&[("type", kind), ("drive", 24.0)])).unwrap();
            let (l, _) = run(&mut sat, &sine(200.0, 0.5, 24_000));
            let tail = &l[12_000..];
            let crest = peak(tail) / rms(tail);
            assert!(crest < 1.3 && peak(tail) < 1.5, "type {kind}: crest {crest}");
        }
    }

    #[test]
    fn utility_gain_width_pan_and_phase() {
        let c: FxContext = ctx();
        let run_on = |p: &[(&str, f64)], l: f32, r: f32| {
            let mut fx = AudioFx::build("utility", SR, &params(p)).unwrap();
            let (mut a, mut b) = (vec![l; 4800], vec![r; 4800]);
            fx.process(&mut a, &mut b, None, &c);
            (a[4799], b[4799])
        };
        let (l, r) = run_on(&[("gain", 6.0)], 0.25, 0.25);
        assert!((l - 0.499).abs() < 0.01 && (r - 0.499).abs() < 0.01);
        let (l, r) = run_on(&[("width", 0.0)], 0.4, 0.0);
        assert!((l - 0.2).abs() < 1e-6 && (r - 0.2).abs() < 1e-6, "width 0 is mono");
        let (l, r) = run_on(&[("pan", -50.0)], 0.3, 0.3);
        assert!((l - 0.3).abs() < 1e-3 && r.abs() < 1e-3, "hard left");
        let (l, r) = run_on(&[("invert", 1.0)], 0.3, 0.3);
        assert!((l + 0.3).abs() < 1e-6 && (r - 0.3).abs() < 1e-6, "left inverted");
    }
}
