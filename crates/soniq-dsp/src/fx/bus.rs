//! Shaping dynamics on busses and busy tracks: a glue compressor, a
//! three-band compressor, a de-esser and a transient shaper. No look-ahead,
//! so no latency.

use std::f32::consts::FRAC_1_SQRT_2;

use super::{Smooth, choice, level_db, time_coef};
use crate::Params;
use crate::common::{Svf, SvfCoef, db_gain};

fn db_to_lin(db: f32) -> f32 {
    (db * 0.115_129_25).exp()
}

fn mix_of(p: &dyn Params, default: f64) -> f32 {
    (p.or("mix", default) / 100.0).clamp(0.0, 1.0) as f32
}

/// Gain change (0 or less, dB) for a level over a threshold, with a 6 dB soft knee.
fn reduction(level: f32, thresh: f32, ratio: f32) -> f32 {
    const KNEE: f32 = 6.0;
    let over = level - thresh;
    let slope = 1.0 / ratio.max(1.0) - 1.0;
    if 2.0 * over.abs() <= KNEE {
        slope * (over + KNEE / 2.0).powi(2) / (2.0 * KNEE)
    } else if over > 0.0 {
        slope * over
    } else {
        0.0
    }
}

const GLUE_RATIOS: [f32; 3] = [2.0, 4.0, 10.0];
const GLUE_ATTACKS: [f64; 7] = [0.01, 0.1, 0.3, 1.0, 3.0, 10.0, 30.0];
const GLUE_RELEASES: [f64; 6] = [100.0, 200.0, 400.0, 600.0, 800.0, 1200.0];

/// A bus compressor in the classic VCA style: few, fixed ratios and times,
/// an automatic release that recovers fast from short peaks and slowly from
/// long ones, a range that limits how far it pulls down, and a soft clipper.
pub struct Glue {
    sr: f32,
    thresh: f32,
    ratio: f32,
    attack: f32,
    /// One release, or (`auto`) a fast and a slow one together.
    release: f32,
    slow: f32,
    auto: bool,
    range: f32,
    makeup: f32,
    mix: f32,
    clip: bool,
    env: f32,
    env_slow: f32,
}

impl Glue {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut glue = Self {
            sr,
            thresh: 0.0,
            ratio: 2.0,
            attack: 0.0,
            release: 0.0,
            slow: 0.0,
            auto: false,
            range: 0.0,
            makeup: 0.0,
            mix: 1.0,
            clip: false,
            env: 0.0,
            env_slow: 0.0,
        };
        glue.configure(p);
        glue
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.thresh = p.or("thresh", -10.0) as f32;
        self.ratio = GLUE_RATIOS[choice(p, "ratio", 1.0).min(GLUE_RATIOS.len() - 1)];
        self.attack = time_coef(GLUE_ATTACKS[choice(p, "attack", 3.0).min(GLUE_ATTACKS.len() - 1)], self.sr);
        let release = choice(p, "release", 6.0);
        self.auto = release >= GLUE_RELEASES.len();
        self.release = time_coef(if self.auto { 100.0 } else { GLUE_RELEASES[release] }, self.sr);
        self.slow = time_coef(1200.0, self.sr);
        self.range = p.or("range", 70.0).max(0.0) as f32;
        self.makeup = p.or("makeup", 0.0) as f32;
        self.mix = mix_of(p, 100.0);
        self.clip = p.or("clip", 0.0) >= 0.5;
    }

    pub fn reduction(&self) -> f32 {
        -self.env.min(self.env_slow)
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let target = reduction(level_db(*l, *r), self.thresh, self.ratio).max(-self.range);
            let coef = if target < self.env { self.attack } else { self.release };
            self.env = target + (self.env - target) * coef;
            // Auto release: a slow follower holds the gain down through long passages.
            let gain_db = if self.auto {
                let coef = if target < self.env_slow { self.attack } else { self.slow };
                self.env_slow = target + (self.env_slow - target) * coef;
                // Short peaks recover fast; what lasts holds the slow one's share.
                self.env.min(0.5 * self.env_slow)
            } else {
                self.env
            };
            let g = db_to_lin(gain_db + self.makeup);
            let (mut a, mut b) = (*l * g, *r * g);
            if self.clip {
                a = soft_clip(a);
                b = soft_clip(b);
            }
            *l = a * self.mix + *l * (1.0 - self.mix);
            *r = b * self.mix + *r * (1.0 - self.mix);
        }
    }

    pub fn adopt(&mut self, old: &mut Glue) {
        (self.env, self.env_slow) = (old.env, old.env_slow);
    }
}

/// Gentle saturation that keeps peaks under 0 dBFS.
fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.5 { x } else { x.signum() * (0.5 + 0.5 * ((x.abs() - 0.5) / 0.5).tanh()) }
}

/// Two Butterworth stages in a row: a fourth-order Linkwitz-Riley split.
/// Its low and high bands add up to the input with every frequency at full
/// level (only the phase turns).
#[derive(Default, Clone, Copy)]
struct Crossover {
    first: Svf,
    low: Svf,
    high: Svf,
}

impl Crossover {
    fn split(&mut self, c: &SvfCoef, x: f32) -> (f32, f32) {
        let (lp, _, hp) = self.first.tick(c, x);
        (self.low.tick(c, lp).0, self.high.tick(c, hp).2)
    }
}

#[derive(Default, Clone, Copy)]
struct Band {
    thresh: f32,
    ratio: f32,
    gain: f32,
    env: f32,
}

/// Each band's threshold, ratio and gain parameters.
const BAND_IDS: [[&str; 3]; 3] = [["lowThresh", "lowRatio", "lowGain"], ["midThresh", "midRatio", "midGain"], ["highThresh", "highRatio", "highGain"]];

impl Band {
    fn configure(&mut self, p: &dyn Params, [thresh, ratio, gain]: [&str; 3]) {
        self.thresh = p.or(thresh, -20.0) as f32;
        self.ratio = p.or(ratio, 3.0).max(1.0) as f32;
        self.gain = db_gain(p.or(gain, 0.0));
    }
}

/// Three bands (split with Linkwitz-Riley crossovers), each compressed on
/// its own and set to its own level: tame a boomy low end without touching
/// the vocals, or bring up the air.
pub struct Multiband {
    sr: f32,
    low_x: SvfCoef,
    high_x: SvfCoef,
    attack: f32,
    release: f32,
    mix: f32,
    bands: [Band; 3],
    /// Per channel: the low/rest split, the mid/high split, and the low band
    /// through the second split's phase so the three add back up.
    splits: [[Crossover; 3]; 2],
}

impl Multiband {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut mb = Self {
            sr,
            low_x: SvfCoef::default(),
            high_x: SvfCoef::default(),
            attack: 0.0,
            release: 0.0,
            mix: 1.0,
            bands: [Band::default(); 3],
            splits: [[Crossover::default(); 3]; 2],
        };
        mb.configure(p);
        mb
    }

    pub fn configure(&mut self, p: &dyn Params) {
        let low = p.or("lowX", 120.0).clamp(30.0, 1000.0) as f32;
        let high = p.or("highX", 2500.0).clamp(f64::from(low) * 1.5, 16_000.0) as f32;
        self.low_x = SvfCoef::with_q(low, FRAC_1_SQRT_2, self.sr);
        self.high_x = SvfCoef::with_q(high, FRAC_1_SQRT_2, self.sr);
        self.attack = time_coef(p.or("attack", 10.0), self.sr);
        self.release = time_coef(p.or("release", 150.0), self.sr);
        self.mix = mix_of(p, 100.0);
        for (band, ids) in self.bands.iter_mut().zip(BAND_IDS) {
            band.configure(p, ids);
        }
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let mut parts = [[0.0f32; 3]; 2];
            for (ch, x) in [*l, *r].into_iter().enumerate() {
                let [first, second, low_phase] = &mut self.splits[ch];
                let (low, rest) = first.split(&self.low_x, x);
                let (mid, high) = second.split(&self.high_x, rest);
                let (a, b) = low_phase.split(&self.high_x, low);
                parts[ch] = [a + b, mid, high];
            }
            let (mut out_l, mut out_r) = (0.0, 0.0);
            for (k, band) in self.bands.iter_mut().enumerate() {
                let target = reduction(level_db(parts[0][k], parts[1][k]), band.thresh, band.ratio);
                let coef = if target < band.env { self.attack } else { self.release };
                band.env = target + (band.env - target) * coef;
                let g = db_to_lin(band.env) * band.gain;
                out_l += parts[0][k] * g;
                out_r += parts[1][k] * g;
            }
            *l = out_l * self.mix + *l * (1.0 - self.mix);
            *r = out_r * self.mix + *r * (1.0 - self.mix);
        }
    }

    pub fn adopt(&mut self, old: &mut Multiband) {
        self.splits = old.splits;
        for (new, old) in self.bands.iter_mut().zip(&old.bands) {
            new.env = old.env;
        }
    }
}

/// Turns down harsh "s" and "t" sounds: the highs above a frequency (or the
/// whole signal) are lowered while they are louder than the threshold.
pub struct DeEsser {
    sr: f32,
    coef: SvfCoef,
    thresh: f32,
    range: f32,
    wide: bool,
    listen: bool,
    filters: [Svf; 2],
    detect: Svf,
    gain: f32,
    attack: f32,
    release: f32,
}

impl DeEsser {
    /// The gain on the sibilance right now (1 when nothing is turned down).
    pub fn gain(&self) -> f32 {
        self.gain
    }

    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut de = Self {
            sr,
            coef: SvfCoef::default(),
            thresh: 0.0,
            range: 0.0,
            wide: false,
            listen: false,
            filters: [Svf::default(); 2],
            detect: Svf::default(),
            gain: 1.0,
            attack: time_coef(0.5, sr),
            release: time_coef(60.0, sr),
        };
        de.configure(p);
        de
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.coef = SvfCoef::with_q(p.or("freq", 6000.0).clamp(1000.0, 16_000.0) as f32, FRAC_1_SQRT_2, self.sr);
        self.thresh = p.or("thresh", -24.0) as f32;
        self.range = db_gain(-p.or("range", 10.0).clamp(0.0, 40.0));
        self.wide = choice(p, "mode", 0.0) == 1;
        self.listen = p.or("listen", 0.0) >= 0.5;
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            // What is above the low part: lowering it leaves the rest exactly as it was.
            let hl = *l - self.filters[0].tick(&self.coef, *l).0;
            let hr = *r - self.filters[1].tick(&self.coef, *r).0;
            let key = self.detect.tick(&self.coef, 0.5 * (*l + *r)).2;
            let over = level_db(key, key) - self.thresh;
            // Down by as much as it is over, no further than the range.
            let target = if over > 0.0 { db_to_lin(-over).max(self.range) } else { 1.0 };
            let coef = if target < self.gain { self.attack } else { self.release };
            self.gain = target + (self.gain - target) * coef;
            if self.listen {
                (*l, *r) = (hl, hr);
            } else if self.wide {
                *l *= self.gain;
                *r *= self.gain;
            } else {
                *l += hl * (self.gain - 1.0);
                *r += hr * (self.gain - 1.0);
            }
        }
    }

    pub fn adopt(&mut self, old: &mut DeEsser) {
        (self.filters, self.detect, self.gain) = (old.filters, old.detect, old.gain);
    }
}

/// More or less punch and more or less ring: a fast and a slow follower of
/// the level tell a hit's start (fast above slow) from its tail (fast
/// below slow), and each gets its own gain.
pub struct TransientShaper {
    attack_amt: f32,
    sustain_amt: f32,
    output: Smooth,
    mix: f32,
    fast: f32,
    slow: f32,
    coefs: [f32; 4],
    gain: f32,
    smooth: f32,
}

impl TransientShaper {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut ts = Self {
            attack_amt: 0.0,
            sustain_amt: 0.0,
            output: Smooth::new(db_gain(p.or("output", 0.0)), sr),
            mix: 1.0,
            fast: 0.0,
            slow: 0.0,
            coefs: [time_coef(0.3, sr), time_coef(30.0, sr), time_coef(15.0, sr), time_coef(250.0, sr)],
            gain: 1.0,
            smooth: time_coef(2.0, sr),
        };
        ts.configure(p);
        ts
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.attack_amt = (p.or("attack", 0.0) / 100.0).clamp(-1.0, 1.0) as f32;
        self.sustain_amt = (p.or("sustain", 0.0) / 100.0).clamp(-1.0, 1.0) as f32;
        self.output.set(db_gain(p.or("output", 0.0)));
        self.mix = mix_of(p, 100.0);
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let [fa, fr, sa, sr] = self.coefs;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let x = l.abs().max(r.abs());
            self.fast = x + (self.fast - x) * if x > self.fast { fa } else { fr };
            self.slow = x + (self.slow - x) * if x > self.slow { sa } else { sr };
            // How far the fast follower is above (a hit) or below (a tail) the slow one, in dB.
            let diff = 20.0 * ((self.fast + 1e-6) / (self.slow + 1e-6)).log10();
            let db = if diff > 0.0 { self.attack_amt * 12.0 * (diff / 6.0).min(1.0) } else { self.sustain_amt * 12.0 * (-diff / 6.0).min(1.0) };
            let target = db_to_lin(db);
            self.gain = target + (self.gain - target) * self.smooth;
            let g = self.gain * self.output.next();
            *l = *l * g * self.mix + *l * (1.0 - self.mix);
            *r = *r * g * self.mix + *r * (1.0 - self.mix);
        }
    }

    pub fn adopt(&mut self, old: &mut TransientShaper) {
        (self.fast, self.slow, self.gain) = (old.fast, old.slow, old.gain);
        self.output.adopt(&old.output);
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;

    fn mixed(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b).map(|(x, y)| x + y).collect()
    }

    /// Level of `hz` in `x` (one bin of a DFT).
    fn tone_level(x: &[f32], hz: f32) -> f32 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, s) in x.iter().enumerate() {
            let w = i as f32 * hz / SR as f32 * std::f32::consts::TAU;
            re += s * w.cos();
            im += s * w.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f32
    }

    #[test]
    fn the_glue_compressor_pulls_loud_passages_down() {
        let p = [("thresh", -20.0), ("ratio", 1.0), ("attack", 2.0), ("release", 0.0)];
        let mut glue = AudioFx::build("glue", SR, &params(&p)).unwrap();
        let (l, _) = run(&mut glue, &sine(200.0, 0.5, 48_000));
        // −6 dBFS peaks, 14 dB over at 4:1: about 10 dB down.
        let out = db(peak(&l[24_000..]));
        assert!(out < -14.0 && out > -19.0, "{out}");
    }

    #[test]
    fn multiband_splits_add_back_up_and_bands_compress_on_their_own() {
        let input = mixed(&sine(60.0, 0.6, 48_000), &sine(5000.0, 0.05, 48_000));
        // Ratios of 1: the three bands add up to the input.
        let mut flat = AudioFx::build("multiband", SR, &params(&[("lowRatio", 1.0), ("midRatio", 1.0), ("highRatio", 1.0)])).unwrap();
        let (l, _) = run(&mut flat, &input);
        assert!((tone_level(&l[24_000..], 60.0) - 0.6).abs() < 0.02 && (tone_level(&l[24_000..], 5000.0) - 0.05).abs() < 0.005);
        // A hard ratio on the low band: the bass comes down, the quiet highs stay.
        let p = [("lowThresh", -30.0), ("lowRatio", 10.0), ("midRatio", 1.0), ("highRatio", 1.0)];
        let mut low = AudioFx::build("multiband", SR, &params(&p)).unwrap();
        let (l, _) = run(&mut low, &input);
        assert!(tone_level(&l[24_000..], 60.0) < 0.15, "{}", tone_level(&l[24_000..], 60.0));
        assert!((tone_level(&l[24_000..], 5000.0) - 0.05).abs() < 0.005);
    }

    #[test]
    fn the_de_esser_lowers_only_the_sibilance() {
        let p = [("freq", 5000.0), ("thresh", -30.0), ("range", 12.0)];
        let mut de = AudioFx::build("deesser", SR, &params(&p)).unwrap();
        let (l, _) = run(&mut de, &mixed(&sine(200.0, 0.3, 48_000), &sine(8000.0, 0.3, 48_000)));
        assert!(tone_level(&l[24_000..], 8000.0) < 0.15, "{}", tone_level(&l[24_000..], 8000.0));
        assert!((tone_level(&l[24_000..], 200.0) - 0.3).abs() < 0.03);
    }

    #[test]
    fn the_transient_shaper_adds_punch_and_cuts_ring() {
        // A hit: a sharp start and a long tail.
        let hit: Vec<f32> = (0..24_000).map(|i| (i as f32 * 0.05).sin() * (-(i as f32) / 6000.0).exp() * 0.5).collect();
        let mut ts = AudioFx::build("transient", SR, &params(&[("attack", 100.0), ("sustain", -100.0)])).unwrap();
        let (l, _) = run(&mut ts, &hit);
        assert!(peak(&l[..480]) > peak(&hit[..480]) * 1.3, "punch {} vs {}", peak(&l[..480]), peak(&hit[..480]));
        assert!(peak(&l[12_000..]) < peak(&hit[12_000..]) * 0.6, "tail {} vs {}", peak(&l[12_000..]), peak(&hit[12_000..]));
    }
}
