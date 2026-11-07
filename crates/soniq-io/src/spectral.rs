//! Short-time Fourier analysis: spectrogram images, spectral edits and
//! profile-based noise reduction.
//!
//! Frames use a periodic Hann window for analysis and synthesis with 75 %
//! overlap, where the squared windows sum to a constant 1.5. An untouched
//! spectrum therefore resynthesises the input exactly, so edits can be spliced
//! back into a longer recording without seams.

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use serde::Deserialize;

use crate::AudioBuffer;

pub const FFT_SIZE: usize = 2048;
const HOP: usize = FFT_SIZE / 4;
/// Sum of the squared Hann windows at 75 % overlap.
const OLA_GAIN: f32 = 1.5;

struct Stft {
    window: Vec<f32>,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
}

impl Stft {
    fn new() -> Self {
        let mut planner = FftPlanner::new();
        Self {
            window: hann(FFT_SIZE),
            forward: planner.plan_fft_forward(FFT_SIZE),
            inverse: planner.plan_fft_inverse(FFT_SIZE),
        }
    }

    /// Frames needed to cover `len` samples, each centred on `t * HOP - FFT_SIZE / 2 + HOP`
    /// after padding; see `frame_start`.
    fn frames(len: usize) -> usize {
        (len + FFT_SIZE) / HOP + 1
    }

    /// First input sample of frame `t` (may be negative: zeros are read there).
    fn frame_start(t: usize) -> isize {
        (t * HOP) as isize - FFT_SIZE as isize + HOP as isize
    }

    fn analyse(&self, input: &[f32], t: usize, buf: &mut [Complex32]) {
        let start = Self::frame_start(t);
        for (i, slot) in buf.iter_mut().enumerate() {
            let at = start + i as isize;
            let x = if at >= 0 && (at as usize) < input.len() { input[at as usize] } else { 0.0 };
            *slot = Complex32::new(x * self.window[i], 0.0);
        }
        self.forward.process(buf);
    }

    /// Analyses every frame of `input`, lets `edit` change its spectrum, and
    /// overlap-adds the result. Only the first `FFT_SIZE / 2 + 1` bins are
    /// meaningful to `edit`; the mirrored half is rebuilt afterwards.
    fn process(&self, input: &[f32], mut edit: impl FnMut(usize, &mut [Complex32])) -> Vec<f32> {
        let len = input.len();
        let mut out = vec![0.0_f32; len];
        let mut buf = vec![Complex32::default(); FFT_SIZE];
        for t in 0..Self::frames(len) {
            self.analyse(input, t, &mut buf);
            edit(t, &mut buf[..=FFT_SIZE / 2]);
            for k in 1..FFT_SIZE / 2 {
                buf[FFT_SIZE - k] = buf[k].conj();
            }
            self.inverse.process(&mut buf);
            let start = Self::frame_start(t);
            let scale = 1.0 / (FFT_SIZE as f32 * OLA_GAIN);
            for (i, c) in buf.iter().enumerate() {
                let at = start + i as isize;
                if at >= 0 && (at as usize) < len {
                    out[at as usize] += c.re * self.window[i] * scale;
                }
            }
        }
        out
    }
}

fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()) as f32).collect()
}

/// Centre sample of frame `t`.
fn frame_centre(t: usize) -> isize {
    Stft::frame_start(t) + (FFT_SIZE / 2) as isize
}

fn bin_hz(k: usize, rate: u32) -> f64 {
    k as f64 * f64::from(rate) / FFT_SIZE as f64
}

// ---------- Spectrogram ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrequencyScale {
    Linear,
    Log,
    Mel,
}

/// Lowest frequency drawn on the log scale.
const LOG_FLOOR_HZ: f64 = 20.0;
/// dB range mapped onto 0–255.
const FLOOR_DB: f32 = -100.0;

impl FrequencyScale {
    /// Frequency at `pos` (0 = bottom, 1 = top) of the display.
    pub fn hz_at(self, pos: f64, nyquist: f64) -> f64 {
        let pos = pos.clamp(0.0, 1.0);
        match self {
            FrequencyScale::Linear => pos * nyquist,
            FrequencyScale::Log => LOG_FLOOR_HZ * (nyquist / LOG_FLOOR_HZ).powf(pos),
            FrequencyScale::Mel => {
                let mel = |f: f64| 2595.0 * (1.0 + f / 700.0).log10();
                700.0 * (10f64.powf(pos * mel(nyquist) / 2595.0) - 1.0)
            }
        }
    }
}

/// Spectrogram of `start..end` frames of every channel: `columns × rows`
/// bytes per channel, row-major from the top (highest frequency) row, each
/// byte mapping −100…0 dBFS onto 0…255.
pub fn spectrogram(buffer: &AudioBuffer, start: usize, end: usize, columns: usize, rows: usize, scale: FrequencyScale) -> Vec<Vec<u8>> {
    let stft = Stft::new();
    let columns = columns.clamp(1, 4096);
    let rows = rows.clamp(8, 2048);
    let end = end.min(buffer.frames()).max(start);
    let nyquist = f64::from(buffer.sample_rate) / 2.0;
    // Full-scale sine → 0 dB.
    let norm = 2.0 / stft.window.iter().sum::<f32>();
    // Each row covers the band between its edges; narrow bands read the nearest bin.
    let edges: Vec<f64> = (0..=rows).map(|r| scale.hz_at(1.0 - r as f64 / rows as f64, nyquist)).collect();
    let bin_of = |hz: f64| (hz * FFT_SIZE as f64 / f64::from(buffer.sample_rate)).round().clamp(0.0, (FFT_SIZE / 2) as f64) as usize;
    let bands: Vec<(usize, usize)> = (0..rows)
        .map(|r| {
            let (hi, lo) = (bin_of(edges[r]), bin_of(edges[r + 1]));
            (lo.min(hi), hi.max(lo))
        })
        .collect();

    let mut buf = vec![Complex32::default(); FFT_SIZE];
    let mut mags = vec![0.0_f32; FFT_SIZE / 2 + 1];
    buffer
        .channels
        .iter()
        .map(|ch| {
            let mut image = vec![0u8; columns * rows];
            let span = (end - start) as f64;
            for c in 0..columns {
                let centre = start as f64 + (c as f64 + 0.5) / columns as f64 * span;
                let first = centre.round() as isize - (FFT_SIZE / 2) as isize;
                for (i, slot) in buf.iter_mut().enumerate() {
                    let at = first + i as isize;
                    let x = if at >= 0 && (at as usize) < ch.len() { ch[at as usize] } else { 0.0 };
                    *slot = Complex32::new(x * stft.window[i], 0.0);
                }
                stft.forward.process(&mut buf);
                for (k, m) in mags.iter_mut().enumerate() {
                    *m = buf[k].norm() * norm;
                }
                for (r, &(lo, hi)) in bands.iter().enumerate() {
                    let peak = mags[lo..=hi].iter().fold(0.0_f32, |a, &b| a.max(b));
                    let db = 20.0 * peak.max(1e-9).log10();
                    image[r * columns + c] = (((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
            image
        })
        .collect()
}

// ---------- Spectral edits ----------

/// Multiplies the band `low_hz..high_hz` by `gain` (linear) for the frames
/// `start..end`, with short ramps at the edges so nothing clicks. Returns a
/// buffer of the same length.
pub fn spectral_gain(buffer: &AudioBuffer, start: usize, end: usize, low_hz: f64, high_hz: f64, gain: f32) -> AudioBuffer {
    let stft = Stft::new();
    let frames = buffer.frames();
    let (start, end) = (start.min(frames), end.min(frames).max(start));
    // Work on the edited part plus one window either side, then splice it in.
    let from = start.saturating_sub(FFT_SIZE);
    let to = (end + FFT_SIZE).min(frames);
    let rate = buffer.sample_rate;
    // Half a window of ramp in time, a few bins in frequency.
    let ramp_t = (FFT_SIZE / 2) as f32;
    let ramp_hz = (bin_hz(3, rate)) as f32;
    let (low, high) = (low_hz.min(high_hz) as f32, low_hz.max(high_hz) as f32);

    let channels = buffer
        .channels
        .iter()
        .map(|ch| {
            let piece = &ch[from..to];
            let edited = stft.process(piece, |t, spectrum| {
                let centre = (frame_centre(t) + from as isize) as f32;
                let inside_t = edge_ramp(centre, start as f32, end as f32, ramp_t);
                if inside_t <= 0.0 {
                    return;
                }
                for (k, bin) in spectrum.iter_mut().enumerate() {
                    let hz = bin_hz(k, rate) as f32;
                    let weight = inside_t * edge_ramp(hz, low, high, ramp_hz);
                    if weight > 0.0 {
                        *bin *= 1.0 + (gain - 1.0) * weight;
                    }
                }
            });
            let mut out = ch.clone();
            out[from..to].copy_from_slice(&edited);
            out
        })
        .collect();
    AudioBuffer::new(rate, channels)
}

/// The gain (dB) of a drawn curve at `hz`: straight lines between points on
/// a log-frequency axis, level beyond the first and last.
pub fn curve_db(points: &[(f64, f64)], hz: f64) -> f64 {
    let Some(first) = points.first() else { return 0.0 };
    let last = points[points.len() - 1];
    if hz <= first.0 {
        return first.1;
    }
    if hz >= last.0 {
        return last.1;
    }
    let i = points.partition_point(|p| p.0 <= hz);
    let (a, b) = (points[i - 1], points[i]);
    let t = (hz.ln() - a.0.ln()) / (b.0.ln() - a.0.ln()).max(1e-12);
    a.1 + (b.1 - a.1) * t
}

/// Filter Curve EQ: a drawn gain curve (Hz, dB), applied with no phase
/// shift through the short-time Fourier transform.
pub fn curve_eq(buffer: &AudioBuffer, points: &[(f64, f64)]) -> AudioBuffer {
    let mut sorted: Vec<(f64, f64)> = points.iter().map(|&(hz, db)| (hz.clamp(10.0, 30_000.0), db.clamp(-30.0, 30.0))).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let rate = buffer.sample_rate;
    let gains: Vec<f32> = (0..=FFT_SIZE / 2).map(|k| 10f64.powf(curve_db(&sorted, bin_hz(k, rate).max(1.0)) / 20.0) as f32).collect();
    let stft = Stft::new();
    let channels = buffer
        .channels
        .iter()
        .map(|ch| {
            stft.process(ch, |_, spectrum| {
                for (bin, g) in spectrum.iter_mut().zip(&gains) {
                    *bin *= *g;
                }
            })
        })
        .collect();
    AudioBuffer::new(rate, channels)
}

/// 1 well inside `lo..hi`, 0 outside, ramping over `ramp` at both edges.
fn edge_ramp(x: f32, lo: f32, hi: f32, ramp: f32) -> f32 {
    let rise = ((x - lo) / ramp + 0.5).clamp(0.0, 1.0);
    let fall = ((hi - x) / ramp + 0.5).clamp(0.0, 1.0);
    rise.min(fall)
}

// ---------- Noise reduction ----------

/// The average noise spectrum of a quiet passage, per bin.
#[derive(Debug, Clone)]
pub struct NoiseProfile {
    pub sample_rate: u32,
    power: Vec<f32>,
    pub seconds: f64,
}

/// Learns the noise floor from a selection that contains only noise.
pub fn noise_profile(buffer: &AudioBuffer) -> Result<NoiseProfile, String> {
    if buffer.frames() < FFT_SIZE {
        return Err("Select at least 0.05 s of noise (half a second or more works best).".into());
    }
    let stft = Stft::new();
    let mut power = vec![0.0_f32; FFT_SIZE / 2 + 1];
    let mut count = 0usize;
    let mut buf = vec![Complex32::default(); FFT_SIZE];
    for ch in &buffer.channels {
        // Only frames that lie fully inside the selection.
        let first = FFT_SIZE / HOP - 1;
        let last = (ch.len().saturating_sub(FFT_SIZE)) / HOP + first;
        for t in first..=last {
            stft.analyse(ch, t, &mut buf);
            for (p, c) in power.iter_mut().zip(&buf) {
                *p += c.norm_sqr();
            }
            count += 1;
        }
    }
    power.iter_mut().for_each(|p| *p /= count.max(1) as f32);
    Ok(NoiseProfile { sample_rate: buffer.sample_rate, power, seconds: buffer.duration() })
}

/// Learns the noise floor without being shown it: the average spectrum of
/// the quietest tenth of the frames (leaving out digital silence). `None`
/// when there is too little audio to tell.
pub fn estimate_noise(buffer: &AudioBuffer) -> Option<NoiseProfile> {
    let stft = Stft::new();
    let first = FFT_SIZE / HOP - 1;
    // Energy of every frame that lies fully inside the audio, from 512-sample blocks.
    let mut frames: Vec<(usize, usize, f64)> = Vec::new();
    for (c, ch) in buffer.channels.iter().enumerate() {
        if ch.len() < FFT_SIZE {
            continue;
        }
        let blocks: Vec<f64> = ch.chunks(HOP).map(|b| b.iter().map(|s| f64::from(*s).powi(2)).sum()).collect();
        let last = (ch.len() - FFT_SIZE) / HOP + first;
        for t in first..=last {
            let b = t - first;
            frames.push((c, t, blocks[b..b + FFT_SIZE / HOP].iter().sum()));
        }
    }
    let floor = 1e-10 * FFT_SIZE as f64;
    let mut energies: Vec<f64> = frames.iter().map(|f| f.2).filter(|e| *e > floor).collect();
    if energies.len() < 40 {
        return None;
    }
    energies.sort_by(f64::total_cmp);
    let cut = energies[(energies.len() - 1) / 10];
    let mut power = vec![0.0_f32; FFT_SIZE / 2 + 1];
    let mut count = 0usize;
    let mut buf = vec![Complex32::default(); FFT_SIZE];
    for &(c, t, e) in &frames {
        if e > floor && e <= cut {
            stft.analyse(&buffer.channels[c], t, &mut buf);
            for (p, x) in power.iter_mut().zip(&buf) {
                *p += x.norm_sqr();
            }
            count += 1;
        }
    }
    power.iter_mut().for_each(|p| *p /= count.max(1) as f32);
    let seconds = count as f64 * HOP as f64 / f64::from(buffer.sample_rate.max(1));
    Some(NoiseProfile { sample_rate: buffer.sample_rate, power, seconds })
}

#[derive(Debug, Clone, Copy)]
pub struct NoiseSettings {
    /// How far noise is pushed down, in dB.
    pub reduction_db: f64,
    /// How far above the noise floor a bin must be to count as signal, in dB.
    pub sensitivity_db: f64,
    /// Neighbouring bins averaged on each side, so the gain mask stays smooth.
    pub smoothing_bins: usize,
}

/// Spectral gating against a noise profile (the approach of Audacity's
/// Noise Reduction): bins near the noise floor are attenuated, with a fast
/// attack and a slow release over time so speech tails are kept.
pub fn reduce_noise(buffer: &AudioBuffer, profile: &NoiseProfile, settings: NoiseSettings) -> AudioBuffer {
    let stft = Stft::new();
    let bins = FFT_SIZE / 2 + 1;
    // The profile may come from audio at another rate: read it by frequency.
    let threshold: Vec<f32> = (0..bins)
        .map(|k| {
            let hz = bin_hz(k, buffer.sample_rate);
            let pk = (hz * FFT_SIZE as f64 / f64::from(profile.sample_rate)).round() as usize;
            profile.power[pk.min(bins - 1)] * 10f64.powf(settings.sensitivity_db / 10.0) as f32
        })
        .collect();
    let floor = -settings.reduction_db.max(0.0) as f32;
    let hop_sec = HOP as f32 / buffer.sample_rate as f32;
    // dB per frame: noise fades back in over 20 ms, gating closes over 150 ms.
    let attack = -floor / (0.02 / hop_sec).max(1.0);
    let release = -floor / (0.15 / hop_sec).max(1.0);
    let lookahead = (0.02 / hop_sec).ceil() as usize;
    let smooth = settings.smoothing_bins;

    let channels = buffer
        .channels
        .iter()
        .map(|ch| {
            // Pass one: which bins of which frames carry signal.
            let frames = Stft::frames(ch.len());
            let mut signal = vec![false; frames * bins];
            let mut buf = vec![Complex32::default(); FFT_SIZE];
            for t in 0..frames {
                stft.analyse(ch, t, &mut buf);
                for k in 0..bins {
                    signal[t * bins + k] = buf[k].norm_sqr() > threshold[k];
                }
            }
            // Pass two: smooth the mask in time and frequency, and apply it.
            let mut held = vec![floor; bins];
            let mut gains_db = vec![0.0_f32; bins];
            let mut gains = vec![0.0_f32; bins];
            stft.process(ch, |t, spectrum| {
                for k in 0..bins {
                    // Attack: open early for signal that is about to start.
                    let ahead = (0..=lookahead)
                        .filter(|j| t + j < frames && signal[(t + j) * bins + k])
                        .map(|j| -(attack * j as f32))
                        .fold(floor, f32::max)
                        .max(floor);
                    // Release: close slowly after the signal stops.
                    held[k] = ahead.max(held[k] - release).min(0.0);
                    gains_db[k] = held[k];
                }
                for (k, gain) in gains.iter_mut().enumerate() {
                    let lo = k.saturating_sub(smooth);
                    let hi = (k + smooth).min(bins - 1);
                    let avg = gains_db[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32;
                    *gain = 10f32.powf(avg / 20.0);
                }
                for (bin, g) in spectrum.iter_mut().zip(&gains) {
                    *bin *= *g;
                }
            })
        })
        .collect();
    AudioBuffer::new(buffer.sample_rate, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(len: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn untouched_spectra_resynthesise_exactly() {
        let input = noise(10_000, 0.5, 7);
        let out = Stft::new().process(&input, |_, _| {});
        let worst = input.iter().zip(&out).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
        assert!(worst < 1e-4, "error {worst}");
    }

    #[test]
    fn spectrogram_puts_a_tone_in_the_right_row() {
        let rate = 48_000;
        let tone: Vec<f32> = (0..rate).map(|i| (2.0 * std::f32::consts::PI * 6500.0 * i as f32 / rate as f32).sin()).collect();
        let image = spectrogram(&AudioBuffer::new(rate as u32, vec![tone]), 0, rate, 4, 24, FrequencyScale::Linear);
        // Rows are 1 kHz bands from the top; 6.5 kHz sits in row 17 (6–7 kHz).
        let column: Vec<u8> = (0..24).map(|r| image[0][r * 4 + 1]).collect();
        let loudest = column.iter().enumerate().max_by_key(|(_, v)| **v).unwrap().0;
        assert_eq!(loudest, 17, "{column:?}");
        assert!(column[17] > 240, "a full-scale tone reads near 0 dB");
    }

    #[test]
    fn spectral_gain_removes_a_band_and_keeps_the_rest() {
        let rate = 48_000u32;
        let n = rate as usize;
        let mix: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                0.5 * (2.0 * std::f32::consts::PI * 300.0 * t).sin() + 0.5 * (2.0 * std::f32::consts::PI * 5000.0 * t).sin()
            })
            .collect();
        let buffer = AudioBuffer::new(rate, vec![mix.clone()]);
        let out = spectral_gain(&buffer, 0, n, 4000.0, 6000.0, 0.0);
        // What is left in the middle is the 300 Hz tone alone.
        let low: Vec<f32> = (0..n).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / rate as f32).sin()).collect();
        let mid = n / 4..3 * n / 4;
        let residual: Vec<f32> = out.channels[0][mid.clone()].iter().zip(&low[mid]).map(|(a, b)| a - b).collect();
        assert!(rms(&residual) < 0.01, "residual {}", rms(&residual));
    }

    #[test]
    fn a_drawn_curve_lowers_and_raises_what_it_says() {
        let rate = 48_000u32;
        let n = rate as usize;
        let tone = |f: f32| -> Vec<f32> { (0..n).map(|i| 0.3 * (2.0 * std::f32::consts::PI * f * i as f32 / rate as f32).sin()).collect() };
        // −12 dB at 100 Hz and below, flat at 1 kHz, +6 dB from 8 kHz.
        let curve = [(100.0, -12.0), (1000.0, 0.0), (8000.0, 6.0)];
        assert!((curve_db(&curve, 316.2) + 6.0).abs() < 0.01, "half way on a log axis");
        let mid = n / 4..3 * n / 4;
        for (f, want) in [(60.0, -12.0), (1000.0, 0.0), (12_000.0, 6.0)] {
            let out = curve_eq(&AudioBuffer::new(rate, vec![tone(f)]), &curve);
            let got = 20.0 * (rms(&out.channels[0][mid.clone()]) / rms(&tone(f)[mid.clone()])).log10();
            assert!((got - want).abs() < 0.5, "{f} Hz: {got} dB");
        }
    }

    #[test]
    fn the_noise_is_learned_from_the_quiet_moments() {
        let rate = 44_100u32;
        let r = rate as usize;
        let mut ch = noise(4 * r, 0.02, 5);
        // Loud sound over the middle half: the quiet tenth is noise alone.
        for (i, s) in ch[r..3 * r].iter_mut().enumerate() {
            *s += 0.5 * (2.0 * std::f32::consts::PI * 700.0 * i as f32 / rate as f32).sin();
        }
        let learned = estimate_noise(&AudioBuffer::new(rate, vec![ch])).unwrap();
        let shown = noise_profile(&AudioBuffer::new(rate, vec![noise(r, 0.02, 6)])).unwrap();
        let total = |p: &NoiseProfile| p.power.iter().sum::<f32>();
        let ratio = total(&learned) / total(&shown);
        assert!((0.5..2.0).contains(&ratio), "learned {ratio}× the noise it was shown");
        assert!(estimate_noise(&AudioBuffer::new(rate, vec![vec![0.1; 1000]])).is_none(), "too short");
    }

    #[test]
    fn noise_reduction_lowers_the_noise_floor() {
        let rate = 44_100u32;
        let hiss = noise(rate as usize * 2, 0.05, 11);
        let profile = noise_profile(&AudioBuffer::new(rate, vec![hiss[..rate as usize].to_vec()])).unwrap();
        let settings = NoiseSettings { reduction_db: 18.0, sensitivity_db: 6.0, smoothing_bins: 3 };
        let out = reduce_noise(&AudioBuffer::new(rate, vec![hiss.clone()]), &profile, settings);
        let before = rms(&hiss[rate as usize..]);
        let after = rms(&out.channels[0][rate as usize..]);
        assert!(after < before * 0.2, "{before} → {after}");
    }
}
