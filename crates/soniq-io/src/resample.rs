//! Offline sample-rate conversion with a polyphase windowed-sinc filter.
//!
//! 32 taps with a Blackman window keep aliasing well below audibility for
//! everyday 44.1 ↔ 48 kHz work. A higher-quality real-time resampler is a
//! later roadmap item; this one runs once per file on import.

use std::f64::consts::PI;

use crate::AudioBuffer;

const TAPS: usize = 32;
const HALF: isize = (TAPS / 2) as isize;
const PHASES: usize = 512;

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 { 1.0 } else { (PI * x).sin() / (PI * x) }
}

fn blackman(x: f64) -> f64 {
    // x in [-TAPS/2, TAPS/2]
    let n = x / TAPS as f64;
    0.42 + 0.5 * (2.0 * PI * n).cos() + 0.08 * (4.0 * PI * n).cos()
}

/// One row of filter weights per fractional phase, each normalised to unity gain.
fn kernel(cutoff: f64) -> Vec<[f32; TAPS]> {
    (0..=PHASES)
        .map(|p| {
            let frac = p as f64 / PHASES as f64;
            let mut row = [0.0_f32; TAPS];
            let mut sum = 0.0;
            for (k, w) in row.iter_mut().enumerate() {
                let x = (k as isize - HALF + 1) as f64 - frac;
                let v = cutoff * sinc(cutoff * x) * blackman(x);
                *w = v as f32;
                sum += v;
            }
            for w in &mut row {
                *w = (f64::from(*w) / sum) as f32;
            }
            row
        })
        .collect()
}

pub fn resample(input: &AudioBuffer, target_rate: u32) -> AudioBuffer {
    if input.sample_rate == target_rate || input.frames() == 0 || target_rate == 0 {
        return AudioBuffer { sample_rate: target_rate.max(1), ..input.clone() };
    }
    let ratio = f64::from(target_rate) / f64::from(input.sample_rate);
    // Below unity the filter must also band-limit to the new Nyquist.
    let table = kernel(ratio.min(1.0) * 0.97);
    let in_frames = input.frames();
    let out_frames = (in_frames as f64 * ratio).round() as usize;

    let channels = input
        .channels
        .iter()
        .map(|src| {
            let mut out = Vec::with_capacity(out_frames);
            for j in 0..out_frames {
                let pos = j as f64 / ratio;
                let base = pos.floor() as isize;
                let frac = pos - base as f64;
                let row = &table[(frac * PHASES as f64).round() as usize];
                let first = base - HALF + 1;
                let mut acc = 0.0_f32;
                if first >= 0 && (first as usize + TAPS) <= in_frames {
                    let window = &src[first as usize..first as usize + TAPS];
                    for (s, w) in window.iter().zip(row) {
                        acc += s * w;
                    }
                } else {
                    // Edges: treat samples outside the file as silence.
                    for (k, w) in row.iter().enumerate() {
                        let i = first + k as isize;
                        if i >= 0 && (i as usize) < in_frames {
                            acc += src[i as usize] * w;
                        }
                    }
                }
                out.push(acc);
            }
            out
        })
        .collect();

    AudioBuffer::new(target_rate, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f64, seconds: f64) -> AudioBuffer {
        let n = (f64::from(rate) * seconds) as usize;
        let ch = (0..n).map(|i| (2.0 * PI * freq * i as f64 / f64::from(rate)).sin() as f32 * 0.5).collect();
        AudioBuffer::new(rate, vec![ch])
    }

    #[test]
    fn keeps_duration_and_level() {
        let input = sine(44_100, 1000.0, 1.0);
        let out = resample(&input, 48_000);
        assert_eq!(out.sample_rate, 48_000);
        assert_eq!(out.frames(), 48_000);
        // Ignore the edges, where the filter sees silence.
        let middle = &out.channels[0][1000..47_000];
        let peak = middle.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak was {peak}");
    }

    #[test]
    fn same_rate_is_untouched() {
        let input = sine(48_000, 440.0, 0.1);
        assert_eq!(resample(&input, 48_000), input);
    }

    #[test]
    fn removes_content_above_the_new_nyquist() {
        // 20 kHz cannot exist at 22.05 kHz; it must be filtered, not aliased.
        let input = sine(48_000, 20_000.0, 0.5);
        let out = resample(&input, 22_050);
        let middle = &out.channels[0][500..out.frames() - 500];
        let peak = middle.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak < 0.05, "aliased peak was {peak}");
    }
}
