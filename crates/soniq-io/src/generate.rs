//! Generators: tones, sweeps, noise, DTMF and silence, as mono audio.

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::AudioBuffer;
use crate::dsp::db_to_gain;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Waveform {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoiseColor {
    White,
    Pink,
    Brown,
}

/// Amplitudes are in dBFS, durations in seconds unless noted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Generator {
    Tone { waveform: Waveform, frequency: f64, amplitude: f64, seconds: f64 },
    /// A sweep from `from` to `to` Hz, exponential so every octave takes as long.
    Chirp { waveform: Waveform, from: f64, to: f64, amplitude: f64, seconds: f64 },
    Noise { color: NoiseColor, amplitude: f64, seconds: f64 },
    /// Telephone keypad tones; `tone` and `gap` in ms.
    Dtmf { digits: String, tone: f64, gap: f64, amplitude: f64 },
    Silence { seconds: f64 },
}

impl Generator {
    pub fn label(&self) -> &'static str {
        match self {
            Generator::Tone { .. } => "Tone",
            Generator::Chirp { .. } => "Chirp",
            Generator::Noise { .. } => "Noise",
            Generator::Dtmf { .. } => "DTMF tones",
            Generator::Silence { .. } => "Silence",
        }
    }
}

/// Longest audio a generator makes, in seconds.
const MAX_SECONDS: f64 = 3600.0;

pub fn generate(generator: &Generator, rate: u32) -> AudioBuffer {
    let sr = f64::from(rate);
    let frames = |seconds: f64| (seconds.clamp(0.0, MAX_SECONDS) * sr).round() as usize;
    let mut out = match generator {
        Generator::Tone { waveform, frequency, amplitude, seconds } => {
            let f = frequency.clamp(1.0, sr / 2.0);
            oscillator(*waveform, frames(*seconds), sr, db_to_gain(*amplitude), |_| f)
        }
        Generator::Chirp { waveform, from, to, amplitude, seconds } => {
            let n = frames(*seconds).max(1);
            let (a, b) = (from.clamp(1.0, sr / 2.0), to.clamp(1.0, sr / 2.0));
            oscillator(*waveform, n, sr, db_to_gain(*amplitude), |i| a * (b / a).powf(i as f64 / n as f64))
        }
        Generator::Noise { color, amplitude, seconds } => noise(*color, frames(*seconds), db_to_gain(*amplitude)),
        Generator::Dtmf { digits, tone, gap, amplitude } => dtmf(digits, frames(tone / 1000.0), frames(gap / 1000.0), sr, db_to_gain(*amplitude)),
        Generator::Silence { seconds } => vec![0.0; frames(*seconds)],
    };
    if !matches!(generator, Generator::Silence { .. } | Generator::Noise { .. } | Generator::Dtmf { .. }) {
        declick(&mut out, (0.002 * sr) as usize);
    }
    AudioBuffer::new(rate, vec![out])
}

/// Band-limited waveforms (PolyBLEP), so square and saw do not alias.
fn oscillator(waveform: Waveform, n: usize, sr: f64, gain: f64, freq_at: impl Fn(usize) -> f64) -> Vec<f32> {
    let blep = |t: f64, dt: f64| {
        if t < dt {
            let x = t / dt;
            x + x - x * x - 1.0
        } else if t > 1.0 - dt {
            let x = (t - 1.0) / dt;
            x * x + x + x + 1.0
        } else {
            0.0
        }
    };
    let mut phase = 0.0_f64;
    // The triangle starts at its trough so it is centred from the first cycle.
    let mut tri = -1.0_f64;
    (0..n)
        .map(|i| {
            let dt = freq_at(i) / sr;
            let v = match waveform {
                Waveform::Sine => (phase * TAU).sin(),
                Waveform::Sawtooth => 2.0 * phase - 1.0 - blep(phase, dt),
                Waveform::Square | Waveform::Triangle => {
                    let sq = if phase < 0.5 { 1.0 } else { -1.0 } + blep(phase, dt) - blep((phase + 0.5) % 1.0, dt);
                    if waveform == Waveform::Square {
                        sq
                    } else {
                        // A leaky integral of the square is a clean triangle.
                        tri = dt * 4.0 * sq + (1.0 - dt * 0.5) * tri;
                        tri
                    }
                }
            };
            phase = (phase + dt) % 1.0;
            (v * gain) as f32
        })
        .collect()
}

fn noise(color: NoiseColor, n: usize, gain: f64) -> Vec<f32> {
    let mut seed = 0x9e37_79b9_u32;
    let mut white = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        f64::from(seed) / f64::from(u32::MAX) * 2.0 - 1.0
    };
    let mut b = [0.0_f64; 7];
    let mut brown = 0.0_f64;
    (0..n)
        .map(|_| {
            let w = white();
            let v = match color {
                NoiseColor::White => w,
                // Paul Kellet's refined pink filter.
                NoiseColor::Pink => {
                    b[0] = 0.99886 * b[0] + w * 0.0555179;
                    b[1] = 0.99332 * b[1] + w * 0.0750759;
                    b[2] = 0.96900 * b[2] + w * 0.1538520;
                    b[3] = 0.86650 * b[3] + w * 0.3104856;
                    b[4] = 0.55000 * b[4] + w * 0.5329522;
                    b[5] = -0.7616 * b[5] - w * 0.0168980;
                    let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
                    b[6] = w * 0.115926;
                    pink * 0.11
                }
                NoiseColor::Brown => {
                    brown = (brown + w * 0.02) * 0.998;
                    brown * 3.5
                }
            };
            (v.clamp(-1.0, 1.0) * gain) as f32
        })
        .collect()
}

/// Keypad frequencies (row, column) for a DTMF digit.
fn dtmf_pair(c: char) -> Option<(f64, f64)> {
    const ROWS: [f64; 4] = [697.0, 770.0, 852.0, 941.0];
    const COLS: [f64; 4] = [1209.0, 1336.0, 1477.0, 1633.0];
    const KEYS: [&str; 4] = ["123A", "456B", "789C", "*0#D"];
    let c = c.to_ascii_uppercase();
    KEYS.iter().enumerate().find_map(|(r, row)| row.find(c).map(|col| (ROWS[r], COLS[col])))
}

fn dtmf(digits: &str, tone: usize, gap: usize, sr: f64, gain: f64) -> Vec<f32> {
    let mut out = Vec::new();
    for (i, c) in digits.chars().filter_map(|c| dtmf_pair(c).map(|p| (c, p))).enumerate() {
        if i > 0 {
            out.extend(std::iter::repeat_n(0.0, gap));
        }
        let (_, (lo, hi)) = c;
        let mut burst: Vec<f32> = (0..tone).map(|k| ((TAU * lo * k as f64 / sr).sin() * 0.5 + (TAU * hi * k as f64 / sr).sin() * 0.5) as f32 * gain as f32).collect();
        declick(&mut burst, (0.002 * sr) as usize);
        out.extend(burst);
    }
    out
}

/// Short linear ramps at both ends.
fn declick(x: &mut [f32], ramp: usize) {
    let ramp = ramp.min(x.len() / 2);
    let n = x.len();
    for i in 0..ramp {
        let g = i as f32 / ramp as f32;
        x[i] *= g;
        x[n - 1 - i] *= g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_has_the_requested_length_and_level() {
        let g = Generator::Tone { waveform: Waveform::Sine, frequency: 1000.0, amplitude: -6.0, seconds: 0.5 };
        let out = generate(&g, 48_000);
        assert_eq!(out.frames(), 24_000);
        assert!((out.peak() - 0.501).abs() < 0.01, "{}", out.peak());
    }

    #[test]
    fn square_stays_within_range() {
        let g = Generator::Tone { waveform: Waveform::Square, frequency: 3000.0, amplitude: 0.0, seconds: 0.1 };
        assert!(generate(&g, 44_100).peak() < 1.3);
    }

    #[test]
    fn dtmf_skips_unknown_characters() {
        let g = Generator::Dtmf { digits: "1-2".into(), tone: 100.0, gap: 50.0, amplitude: -6.0 };
        assert_eq!(generate(&g, 8000).frames(), 800 + 400 + 800);
    }

    #[test]
    fn generators_deserialise_from_ui_json() {
        let g: Generator = serde_json::from_str(r#"{"type":"noise","color":"pink","amplitude":-12,"seconds":2}"#).unwrap();
        assert_eq!(g, Generator::Noise { color: NoiseColor::Pink, amplitude: -12.0, seconds: 2.0 });
    }
}
