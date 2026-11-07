//! Modulation: chorus, phaser or flanger, and an auto filter, each moved by
//! its own LFO (the right channel a little behind the left, for width).

use std::f32::consts::PI;

use super::{DelayLine, choice};
use crate::Params;
use crate::common::{Svf, SvfCoef, sin_turns};

/// Samples between updates of swept filter coefficients.
const CONTROL: usize = 16;

fn lfo_step(p: &dyn Params, default_hz: f64, sr: f32) -> f32 {
    p.or("rate", default_hz).clamp(0.01, 40.0) as f32 / sr
}

fn mix_of(p: &dyn Params, default: f64) -> f32 {
    (p.or("mix", default) / 100.0).clamp(0.0, 1.0) as f32
}

pub struct Chorus {
    sr: f32,
    lines: [DelayLine; 2],
    phase: f32,
    step: f32,
    base: f32,
    depth: f32,
    mix: f32,
}

impl Chorus {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let lines = [DelayLine::new(0.05, sr), DelayLine::new(0.05, sr)];
        let mut chorus = Self { sr, lines, phase: 0.0, step: 0.0, base: 0.012 * sr, depth: 0.0, mix: 0.0 };
        chorus.configure(p);
        chorus
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.step = lfo_step(p, 0.8, self.sr);
        self.depth = (p.or("depth", 50.0) / 100.0).clamp(0.0, 1.0) as f32 * 0.006 * self.sr;
        self.mix = mix_of(p, 50.0);
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.lines[0].push(*l);
            self.lines[1].push(*r);
            let wl = self.lines[0].read(self.base + self.depth * sin_turns(self.phase));
            let wr = self.lines[1].read(self.base + self.depth * sin_turns(self.phase + 0.25));
            self.phase = (self.phase + self.step).fract();
            *l = *l * (1.0 - self.mix) + wl * self.mix;
            *r = *r * (1.0 - self.mix) + wr * self.mix;
        }
    }

    pub fn adopt(&mut self, old: &mut Chorus) {
        for (new, old) in self.lines.iter_mut().zip(old.lines.iter_mut()) {
            new.adopt(old);
        }
        self.phase = old.phase;
    }
}

const STAGES: usize = 6;

/// A phaser (six all-pass stages swept between 200 Hz and 6 kHz) or a
/// flanger (a 0.5–8 ms delay), both with feedback.
pub struct Phaser {
    flanger: bool,
    phase: f32,
    step: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
    sr: f32,
    stages: [[f32; STAGES]; 2],
    coef: [f32; 2],
    last: [f32; 2],
    lines: [DelayLine; 2],
    countdown: usize,
}

impl Phaser {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut phaser = Self {
            flanger: false,
            phase: 0.0,
            step: 0.0,
            depth: 0.0,
            feedback: 0.0,
            mix: 0.0,
            sr,
            stages: [[0.0; STAGES]; 2],
            coef: [0.0; 2],
            last: [0.0; 2],
            lines: [DelayLine::new(0.02, sr), DelayLine::new(0.02, sr)],
            countdown: 0,
        };
        phaser.configure(p);
        phaser
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.flanger = choice(p, "mode", 0.0) == 1;
        self.step = lfo_step(p, 0.5, self.sr);
        self.depth = (p.or("depth", 70.0) / 100.0).clamp(0.0, 1.0) as f32;
        self.feedback = (p.or("fb", 40.0) / 100.0).clamp(0.0, 0.95) as f32;
        self.mix = mix_of(p, 50.0);
    }

    /// LFO position 0–1 for a channel.
    fn sweep(&self, ch: usize) -> f32 {
        sin_turns(self.phase + ch as f32 * 0.15) * 0.5 + 0.5
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            if self.countdown == 0 && !self.flanger {
                for ch in 0..2 {
                    let f = 200.0 * 30f32.powf(self.sweep(ch) * self.depth);
                    let t = (PI * f / self.sr).tan();
                    self.coef[ch] = (t - 1.0) / (t + 1.0);
                }
                self.countdown = CONTROL;
            }
            self.countdown = self.countdown.saturating_sub(1);
            let mut out = [0.0_f32; 2];
            for (ch, x) in [*l, *r].into_iter().enumerate() {
                let wet = if self.flanger {
                    let delay = (0.0005 + 0.0075 * self.sweep(ch) * self.depth) * self.sr;
                    self.lines[ch].push(x + self.last[ch] * self.feedback);
                    self.lines[ch].read(delay)
                } else {
                    let a = self.coef[ch];
                    let mut y = x + self.last[ch] * self.feedback;
                    for z in &mut self.stages[ch] {
                        let out = a * y + *z;
                        *z = y - a * out;
                        y = out;
                    }
                    y
                };
                self.last[ch] = wet;
                out[ch] = x * (1.0 - self.mix) + wet * self.mix;
            }
            self.phase = (self.phase + self.step).fract();
            [*l, *r] = out;
        }
    }

    pub fn adopt(&mut self, old: &mut Phaser) {
        self.phase = old.phase;
        self.stages = old.stages;
        self.last = old.last;
        for (new, old) in self.lines.iter_mut().zip(old.lines.iter_mut()) {
            new.adopt(old);
        }
    }
}

/// A resonant low-pass, high-pass or band-pass whose cutoff an LFO sweeps.
pub struct AutoFilter {
    kind: usize,
    cutoff: f32,
    res: f32,
    /// Sweep depth in octaves either way.
    amount: f32,
    phase: f32,
    step: f32,
    mix: f32,
    sr: f32,
    filters: [Svf; 2],
    coef: SvfCoef,
    countdown: usize,
}

impl AutoFilter {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut filter = Self {
            kind: 0,
            cutoff: 1200.0,
            res: 0.0,
            amount: 0.0,
            phase: 0.0,
            step: 0.0,
            mix: 1.0,
            sr,
            filters: [Svf::default(); 2],
            coef: SvfCoef::default(),
            countdown: 0,
        };
        filter.configure(p);
        filter
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.kind = choice(p, "type", 0.0).min(2);
        self.cutoff = p.or("cutoff", 1200.0).clamp(20.0, 20_000.0) as f32;
        self.res = (p.or("res", 30.0) / 100.0).clamp(0.0, 0.95) as f32;
        self.amount = (p.or("lfoAmt", 0.0) / 100.0).clamp(0.0, 1.0) as f32 * 3.0;
        self.step = lfo_step(p, 1.0, self.sr);
        self.mix = mix_of(p, 100.0);
        self.coef = SvfCoef::new(self.cutoff, self.res, self.sr);
        self.countdown = 0;
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            if self.countdown == 0 {
                if self.amount > 0.0 {
                    self.coef = SvfCoef::new(self.cutoff * (sin_turns(self.phase) * self.amount).exp2(), self.res, self.sr);
                }
                self.countdown = CONTROL;
            }
            self.countdown -= 1;
            self.phase = (self.phase + self.step).fract();
            for (x, f) in [&mut *l, &mut *r].into_iter().zip(self.filters.iter_mut()) {
                let (low, band, high) = f.tick(&self.coef, *x);
                let y = match self.kind {
                    0 => low,
                    1 => high,
                    _ => band,
                };
                *x = *x * (1.0 - self.mix) + y * self.mix;
            }
        }
    }

    pub fn adopt(&mut self, old: &mut AutoFilter) {
        self.filters = old.filters;
        self.phase = old.phase;
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;

    #[test]
    fn modulation_effects_change_the_sound_and_dry_passes_untouched() {
        let input = sine(440.0, 0.5, 24_000);
        for kind in ["chorus", "phaser"] {
            for mode in [0.0, 1.0] {
                let mut fx = AudioFx::build(kind, SR, &params(&[("mode", mode), ("mix", 50.0)])).unwrap();
                let (l, r) = run(&mut fx, &input);
                assert!(l.iter().zip(&input).any(|(a, b)| (a - b).abs() > 0.01), "{kind} changes the sound");
                assert!(peak(&l) < 1.0 && l != r, "{kind}: bounded and wide");
                let mut dry = AudioFx::build(kind, SR, &params(&[("mode", mode), ("mix", 0.0)])).unwrap();
                assert_eq!(run(&mut dry, &input).0, input);
            }
        }
    }

    #[test]
    fn the_flanger_adds_a_short_delayed_copy() {
        let mut fx = AudioFx::build("phaser", SR, &params(&[("mode", 1.0), ("depth", 0.0), ("fb", 0.0), ("mix", 50.0)])).unwrap();
        let mut x = vec![0.0; 2400];
        x[0] = 1.0;
        let (l, _) = run(&mut fx, &x);
        // With no sweep the delay sits at 0.5 ms: 24 samples.
        assert!((l[24] - 0.5).abs() < 0.05, "{}", l[24]);
    }

    #[test]
    fn the_auto_filter_low_passes() {
        let p = [("type", 0.0), ("cutoff", 500.0), ("res", 0.0), ("lfoAmt", 0.0)];
        let gain = |hz: f32| {
            let mut fx = AudioFx::build("autofilter", SR, &params(&p)).unwrap();
            let input = sine(hz, 0.5, 24_000);
            db(rms(&run(&mut fx, &input).0[12_000..])) - db(rms(&input[12_000..]))
        };
        assert!(gain(5000.0) < -30.0 && gain(100.0).abs() < 1.0, "{} {}", gain(5000.0), gain(100.0));
    }
}
