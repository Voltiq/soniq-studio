//! A vocoder: the track's own sound (a voice, a drum loop) is split into
//! bands and their levels shape the same bands of a carrier: another track
//! (the sidechain, say a pad), noise (a whisper) or a buzzing saw.

use super::{Sidechain, Smooth, choice, time_coef};
use crate::Params;
use crate::common::{Noise, Svf, SvfCoef, db_gain, midi_hz};

/// Most bands; the band choice picks 8, 16 or this many.
const MAX_BANDS: usize = 24;
const BAND_CHOICES: [usize; 3] = [8, 16, 24];
const LOWEST: f32 = 120.0;
const HIGHEST: f32 = 8000.0;
/// Brings the pink noise carrier to about the saw's level in the bands.
const PINK_GAIN: f32 = 0.55;

/// Pink noise (Paul Kellet's three-pole filter on white): as much in every
/// octave, so each band of the vocoder gets the same share.
#[derive(Clone, Copy, Default)]
struct Pink([f32; 3]);

impl Pink {
    fn next(&mut self, white: f32) -> f32 {
        let b = &mut self.0;
        b[0] = 0.99765 * b[0] + white * 0.099_046;
        b[1] = 0.963 * b[1] + white * 0.296_516_4;
        b[2] = 0.57 * b[2] + white * 1.052_691_3;
        (b[0] + b[1] + b[2] + white * 0.1848) * PINK_GAIN
    }
}

#[derive(Clone, Copy)]
enum Carrier {
    Sidechain,
    Noise,
    Saw,
}

pub struct Vocoder {
    sr: f32,
    bands: usize,
    /// Analysis bands (on the input) and synthesis bands (on the carrier, maybe shifted).
    analysis: [SvfCoef; MAX_BANDS],
    synthesis: [SvfCoef; MAX_BANDS],
    reading: [Svf; MAX_BANDS],
    shaping: [[Svf; MAX_BANDS]; 2],
    levels: [f32; MAX_BANDS],
    attack: f32,
    release: f32,
    carrier: Carrier,
    saw_step: f32,
    saw: f32,
    noise: Noise,
    pink: Pink,
    output: Smooth,
    mix: f32,
}

impl Vocoder {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut v = Self {
            sr,
            bands: 16,
            analysis: [SvfCoef::default(); MAX_BANDS],
            synthesis: [SvfCoef::default(); MAX_BANDS],
            reading: [Svf::default(); MAX_BANDS],
            shaping: [[Svf::default(); MAX_BANDS]; 2],
            levels: [0.0; MAX_BANDS],
            attack: 0.0,
            release: 0.0,
            carrier: Carrier::Noise,
            saw_step: 0.0,
            saw: 0.0,
            noise: Noise(0x2545_f491),
            pink: Pink::default(),
            output: Smooth::new(db_gain(p.or("output", 6.0)), sr),
            mix: 1.0,
        };
        v.configure(p);
        v
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.bands = BAND_CHOICES[choice(p, "bands", 1.0).min(BAND_CHOICES.len() - 1)];
        let shift = 2f32.powf((p.or("formant", 0.0).clamp(-12.0, 12.0) / 12.0) as f32);
        // Log-spaced centres; each band as wide as the spacing.
        let octaves = (HIGHEST / LOWEST).log2();
        let width = octaves / self.bands as f32;
        let q = 1.0 / (2.0 * (std::f32::consts::LN_2 / 2.0 * width).sinh());
        for k in 0..self.bands {
            let centre = LOWEST * 2f32.powf(width * (k as f32 + 0.5));
            self.analysis[k] = SvfCoef::with_q(centre, q, self.sr);
            self.synthesis[k] = SvfCoef::with_q(centre * shift, q, self.sr);
        }
        self.attack = time_coef(p.or("attack", 5.0), self.sr);
        self.release = time_coef(p.or("release", 80.0), self.sr);
        self.carrier = match choice(p, "carrier", 1.0) {
            0 => Carrier::Sidechain,
            1 => Carrier::Noise,
            _ => Carrier::Saw,
        };
        self.saw_step = midi_hz(p.or("pitch", 48.0).clamp(12.0, 96.0) as f32) / self.sr;
        self.output.set(db_gain(p.or("output", 6.0)));
        self.mix = (p.or("mix", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], sidechain: Sidechain<'_>) {
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let input = 0.5 * (*l + *r);
            // The carrier: the sidechain track when there is one, else the built-in sound.
            let carrier = match (self.carrier, sidechain) {
                (Carrier::Sidechain, Some((kl, kr))) => [kl[i], kr[i]],
                (Carrier::Noise, _) => {
                    let n = self.pink.next(self.noise.next());
                    [n, n]
                }
                _ => {
                    self.saw = (self.saw + self.saw_step).fract();
                    let s = 2.0 * self.saw - 1.0;
                    [s, s]
                }
            };
            let mut out = [0.0f32; 2];
            for k in 0..self.bands {
                let a = &self.analysis[k];
                // Unity at the centre: band output times the damping.
                let level = (self.reading[k].tick(a, input).1 * a.k()).abs();
                let coef = if level > self.levels[k] { self.attack } else { self.release };
                self.levels[k] = level + (self.levels[k] - level) * coef;
                let s = &self.synthesis[k];
                for (ch, o) in out.iter_mut().enumerate() {
                    *o += self.shaping[ch][k].tick(s, carrier[ch]).1 * s.k() * self.levels[k];
                }
            }
            // About the input's own level at the default +6 dB.
            let g = self.output.next() * 0.5;
            *l = out[0] * g * self.mix + *l * (1.0 - self.mix);
            *r = out[1] * g * self.mix + *r * (1.0 - self.mix);
        }
    }

    pub fn adopt(&mut self, old: &mut Vocoder) {
        (self.reading, self.shaping, self.levels) = (old.reading, old.shaping, old.levels);
        (self.saw, self.noise, self.pink) = (old.saw, Noise(old.noise.0), old.pink);
        self.output.adopt(&old.output);
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;

    #[test]
    fn the_carrier_speaks_only_while_the_input_does() {
        let mut vocoder = AudioFx::build("vocoder", SR, &params(&[("carrier", 2.0)])).unwrap();
        // A tone for half a second, then a second of silence.
        let mut input = sine(220.0, 0.3, 24_000);
        input.extend(vec![0.0; 48_000]);
        let (l, _) = run(&mut vocoder, &input);
        // About as loud as what it hears.
        let level = rms(&l[12_000..24_000]);
        assert!(level > 0.1 && level < 0.4, "{level}");
        // Its bands let go (80 ms release) once the input stops.
        assert!(rms(&l[60_000..72_000]) < level * 0.01, "{}", rms(&l[60_000..72_000]));
        // A sidechain carrier: silent key, silent output.
        let mut keyed = AudioFx::build("vocoder", SR, &params(&[("carrier", 0.0)])).unwrap();
        let silence = vec![0.0f32; 24_000];
        let (l, _) = run_keyed(&mut keyed, &sine(220.0, 0.3, 24_000), Some(&silence));
        assert!(peak(&l) < 1e-6);
    }

    #[test]
    fn noise_speaks_about_as_loud_as_the_saw() {
        // A broad input, like a drum loop: decaying noise bursts, low ones loudest.
        let mut seed = 0x1234_5678u32;
        let input: Vec<f32> = (0..48_000)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let white = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
                let hit = (i % 12_000) as f32;
                (0.4 * (-hit / 2000.0).exp() * white) + 0.5 * (-hit / 4000.0).exp() * (hit * std::f32::consts::TAU * 60.0 / 48_000.0).sin()
            })
            .collect();
        let level = |carrier: f64| {
            let mut vocoder = AudioFx::build("vocoder", SR, &params(&[("carrier", carrier)])).unwrap();
            rms(&run(&mut vocoder, &input).0[12_000..])
        };
        let (noise, saw) = (level(1.0), level(2.0));
        let db = 20.0 * (noise / saw).log10();
        assert!(db.abs() < 4.0, "noise {noise} saw {saw}: {db:.1} dB");
    }
}
