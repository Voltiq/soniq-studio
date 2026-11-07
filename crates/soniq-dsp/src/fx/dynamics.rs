//! Dynamics: a compressor and a gate (either can listen to another track's
//! signal, the sidechain), and a brick-wall limiter. None of them looks
//! ahead, so none adds latency.

use super::{Sidechain, Smooth, level_db, time_coef};
use crate::Params;
use crate::common::db_gain;

fn db_to_lin(db: f32) -> f32 {
    (db * 0.115_129_25).exp()
}

/// The key level for sample `i`: the sidechain when there is one, else the input itself.
fn key_level(sidechain: Sidechain<'_>, i: usize, l: f32, r: f32) -> f32 {
    match sidechain {
        Some((kl, kr)) => level_db(kl[i], kr[i]),
        None => level_db(l, r),
    }
}

pub struct Compressor {
    sr: f32,
    thresh: f32,
    ratio: f32,
    knee: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    mix: f32,
    /// Gain change in dB (0 or less), following the key with attack and release.
    env: f32,
}

impl Compressor {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut comp = Self { sr, thresh: 0.0, ratio: 1.0, knee: 0.0, attack: 0.0, release: 0.0, makeup: 0.0, mix: 1.0, env: 0.0 };
        comp.configure(p);
        comp
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.thresh = p.or("thresh", -18.0) as f32;
        self.ratio = p.or("ratio", 4.0).max(1.0) as f32;
        self.knee = p.or("knee", 6.0).max(0.0) as f32;
        self.attack = time_coef(p.or("attack", 10.0), self.sr);
        self.release = time_coef(p.or("release", 120.0), self.sr);
        self.makeup = p.or("makeup", 0.0) as f32;
        self.mix = (p.or("mix", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    }

    /// The static curve: gain change in dB for a key level in dB, with a soft knee.
    fn curve(&self, level: f32) -> f32 {
        let over = level - self.thresh;
        let slope = 1.0 / self.ratio - 1.0;
        if self.knee > 0.0 && 2.0 * over.abs() <= self.knee {
            slope * (over + self.knee / 2.0).powi(2) / (2.0 * self.knee)
        } else if over > 0.0 {
            slope * over
        } else {
            0.0
        }
    }

    /// How far the gain is pulled down right now, in dB (positive).
    pub fn reduction(&self) -> f32 {
        -self.env
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], sidechain: Sidechain<'_>) {
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let target = self.curve(key_level(sidechain, i, *l, *r));
            let coef = if target < self.env { self.attack } else { self.release };
            self.env = target + (self.env - target) * coef;
            let g = db_to_lin(self.env + self.makeup);
            let wet = g * self.mix + (1.0 - self.mix);
            *l *= wet;
            *r *= wet;
        }
    }

    pub fn adopt(&mut self, old: &mut Compressor) {
        self.env = old.env;
    }
}

pub struct Gate {
    sr: f32,
    open_at: f32,
    /// Hysteresis: once open, the gate closes only below this.
    close_at: f32,
    floor: f32,
    attack: f32,
    release: f32,
    hold: u32,
    held: u32,
    open: bool,
    gain: f32,
}

impl Gate {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut gate = Self { sr, open_at: 0.0, close_at: 0.0, floor: 0.0, attack: 0.0, release: 0.0, hold: 0, held: 0, open: false, gain: 0.0 };
        gate.configure(p);
        gate
    }

    pub fn configure(&mut self, p: &dyn Params) {
        let thresh = p.or("thresh", -40.0) as f32;
        self.open_at = thresh;
        self.close_at = thresh - 3.0;
        self.floor = db_gain(p.or("range", -80.0));
        self.attack = time_coef(p.or("attack", 0.5), self.sr);
        self.release = time_coef(p.or("release", 80.0), self.sr);
        self.hold = (p.or("hold", 20.0).max(0.0) * 0.001 * f64::from(self.sr)) as u32;
        self.held = self.held.min(self.hold);
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], sidechain: Sidechain<'_>) {
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let level = key_level(sidechain, i, *l, *r);
            if level > self.open_at {
                self.open = true;
                self.held = self.hold;
            } else if self.held > 0 {
                self.held -= 1;
            } else if level < self.close_at {
                self.open = false;
            }
            let target = if self.open { 1.0 } else { self.floor };
            let coef = if target > self.gain { self.attack } else { self.release };
            self.gain = target + (self.gain - target) * coef;
            *l *= self.gain;
            *r *= self.gain;
        }
    }

    pub fn adopt(&mut self, old: &mut Gate) {
        (self.open, self.held, self.gain) = (old.open, old.held.min(self.hold), old.gain);
    }
}

pub struct Limiter {
    sr: f32,
    input: Smooth,
    ceiling: f32,
    release: f32,
    /// Gain applied right now (1 when nothing is limited).
    env: f32,
}

impl Limiter {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut lim = Self { sr, input: Smooth::new(db_gain(p.or("gain", 0.0)), sr), ceiling: 1.0, release: 0.0, env: 1.0 };
        lim.configure(p);
        lim
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.input.set(db_gain(p.or("gain", 0.0)));
        self.ceiling = db_gain(p.or("ceiling", -1.0).min(0.0));
        self.release = time_coef(p.or("release", 50.0), self.sr);
    }

    pub fn reduction(&self) -> f32 {
        -20.0 * self.env.max(1e-6).log10()
    }

    /// Instant attack, so nothing ever passes the ceiling; smooth release.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let g = self.input.next();
            let (a, b) = (*l * g, *r * g);
            let peak = a.abs().max(b.abs());
            let target = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
            self.env = if target < self.env { target } else { target + (self.env - target) * self.release };
            *l = (a * self.env).clamp(-self.ceiling, self.ceiling);
            *r = (b * self.env).clamp(-self.ceiling, self.ceiling);
        }
    }

    pub fn adopt(&mut self, old: &mut Limiter) {
        self.env = old.env;
        self.input.adopt(&old.input);
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;

    #[test]
    fn the_compressor_follows_its_ratio_above_the_threshold() {
        let p = [("thresh", -20.0), ("ratio", 4.0), ("knee", 0.0), ("attack", 1.0), ("release", 50.0), ("makeup", 0.0)];
        let mut comp = AudioFx::build("comp", SR, &params(&p)).unwrap();
        // −6 dBFS peaks: 14 dB over, so 10.5 dB of reduction.
        let (l, _) = run(&mut comp, &sine(200.0, 0.5, 48_000));
        let out = db(peak(&l[24_000..]));
        assert!((out + 16.5).abs() < 1.5, "{out}");
        // Below the threshold nothing changes.
        let mut comp = AudioFx::build("comp", SR, &params(&p)).unwrap();
        let quiet = sine(200.0, 0.05, 48_000);
        let (l, _) = run(&mut comp, &quiet);
        assert!((db(peak(&l[24_000..])) - db(0.05)).abs() < 0.1);
    }

    #[test]
    fn a_sidechain_ducks_a_quiet_signal() {
        let p = [("thresh", -20.0), ("ratio", 8.0), ("attack", 1.0), ("release", 50.0), ("makeup", 0.0)];
        let mut comp = AudioFx::build("comp", SR, &params(&p)).unwrap();
        let quiet = sine(300.0, 0.05, 48_000);
        let kick = sine(60.0, 0.9, 48_000);
        let (l, _) = run_keyed(&mut comp, &quiet, Some(&kick));
        assert!(db(peak(&l[24_000..])) < db(0.05) - 10.0, "{}", db(peak(&l[24_000..])));
    }

    #[test]
    fn the_gate_closes_below_its_threshold() {
        let p = [("thresh", -40.0), ("range", -80.0), ("hold", 5.0), ("release", 10.0)];
        let mut gate = AudioFx::build("gate", SR, &params(&p)).unwrap();
        let (l, _) = run(&mut gate, &sine(200.0, 0.001, 24_000));
        assert!(peak(&l[12_000..]) < 1e-6);
        let mut gate = AudioFx::build("gate", SR, &params(&p)).unwrap();
        let (l, _) = run(&mut gate, &sine(200.0, 0.5, 24_000));
        assert!(peak(&l[12_000..]) > 0.49);
    }

    #[test]
    fn the_limiter_never_passes_its_ceiling() {
        let p = [("gain", 12.0), ("ceiling", -1.0), ("release", 30.0)];
        let mut lim = AudioFx::build("limiter", SR, &params(&p)).unwrap();
        let (l, _) = run(&mut lim, &sine(100.0, 1.0, 48_000));
        assert!(peak(&l) <= 0.891_3, "{}", peak(&l));
        assert!(peak(&l[24_000..]) > 0.85);
    }
}
