//! Eight-band parametric EQ. Each band is a low or high cut (24 dB/oct), a
//! shelf, a bell or a notch, from the RBJ cookbook. The UI draws the same
//! curves (src/state/eqCurve.ts).

use std::f64::consts::PI;

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
}

impl Eq {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut eq = Self { sr: f64::from(sr), bands: [Band::default(); BANDS], output: Smooth::new(db_gain(p.or("output", 0.0)), sr) };
        eq.configure(p);
        eq
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
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
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
    fn shelves_and_notches() {
        let shelf = [("b7freq", 4000.0), ("b7gain", -6.0)];
        assert!((gain_at(&shelf, 16_000.0) + 6.0).abs() < 0.5);
        assert!(gain_at(&shelf, 200.0).abs() < 0.3);
        let notch = [("b4type", 3.0), ("b4freq", 1000.0), ("b4q", 4.0)];
        assert!(gain_at(&notch, 1000.0) < -30.0);
    }
}
