//! Audacity's Analyze menu: a spectrum plot, the levels for a contrast
//! check, and finding clipped stretches, beats and sounds to mark.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use serde::Serialize;

use crate::AudioBuffer;

/// An averaged spectrum: dB at each of `size / 2 + 1` bins, `sample_rate / size` Hz apart.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumPlot {
    pub sample_rate: u32,
    pub size: usize,
    pub db: Vec<f32>,
    /// The loudest bin's frequency, refined between bins.
    pub peak_hz: f64,
}

/// Welch's method: Hann windows of `size`, half overlapping, power averaged
/// over every window and channel; a full-scale sine reads 0 dB.
pub fn plot_spectrum(buffer: &AudioBuffer, size: usize) -> Result<SpectrumPlot, String> {
    let size = size.clamp(128, 65_536).next_power_of_two();
    if buffer.frames() < size {
        return Err(format!("Select at least {size} samples ({:.2} s) to plot the spectrum at this size.", size as f64 / f64::from(buffer.sample_rate)));
    }
    let fft: Arc<dyn Fft<f32>> = FftPlanner::new().plan_fft_forward(size);
    let window: Vec<f32> = (0..size).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / size as f64).cos()) as f32).collect();
    let mut power = vec![0.0_f64; size / 2 + 1];
    let mut count = 0usize;
    let mut buf = vec![Complex::<f32>::default(); size];
    for ch in &buffer.channels {
        let mut start = 0;
        while start + size <= ch.len() {
            for ((slot, s), w) in buf.iter_mut().zip(&ch[start..start + size]).zip(&window) {
                *slot = Complex::new(s * w, 0.0);
            }
            fft.process(&mut buf);
            for (p, c) in power.iter_mut().zip(&buf) {
                *p += f64::from(c.norm_sqr());
            }
            count += 1;
            start += size / 2;
        }
    }
    let scale = (4.0 / size as f64).powi(2) / count.max(1) as f64;
    let db: Vec<f32> = power.iter().map(|p| (10.0 * (p * scale).max(1e-14).log10()) as f32).collect();
    // The peak, refined with a parabola through it and its neighbours.
    let k = (1..db.len() - 1).max_by(|&a, &b| db[a].total_cmp(&db[b])).unwrap_or(1);
    let (a, b, c) = (f64::from(db[k - 1]), f64::from(db[k]), f64::from(db[k + 1]));
    let shift = if (a - 2.0 * b + c).abs() > 1e-12 { 0.5 * (a - c) / (a - 2.0 * b + c) } else { 0.0 };
    let peak_hz = (k as f64 + shift.clamp(-0.5, 0.5)) * f64::from(buffer.sample_rate) / size as f64;
    Ok(SpectrumPlot { sample_rate: buffer.sample_rate, size, db, peak_hz })
}

/// The RMS level, all channels together, in dBFS (−∞ reads as −140).
pub fn rms_db(buffer: &AudioBuffer) -> f64 {
    let n = buffer.frames() * buffer.channel_count();
    let sum: f64 = buffer.channels.iter().flatten().map(|s| f64::from(*s).powi(2)).sum();
    if n == 0 || sum <= 0.0 { -140.0 } else { 10.0 * (sum / n as f64).log10() }
}

/// Runs of at least `min_run` samples at or above `level` (in any channel),
/// as (start, end) frames; runs closer than 10 ms are joined.
pub fn find_clipping(buffer: &AudioBuffer, level: f32, min_run: usize) -> Vec<(usize, usize)> {
    let n = buffer.frames();
    let join = (buffer.sample_rate / 100) as usize;
    let mut found: Vec<(usize, usize)> = Vec::new();
    for ch in &buffer.channels {
        let mut i = 0;
        while i < n {
            if ch[i].abs() < level {
                i += 1;
                continue;
            }
            let start = i;
            while i < n && ch[i].abs() >= level {
                i += 1;
            }
            if i - start >= min_run.max(1) {
                found.push((start, i));
            }
        }
    }
    found.sort_unstable();
    let mut joined: Vec<(usize, usize)> = Vec::new();
    for (a, b) in found {
        match joined.last_mut() {
            Some(last) if a <= last.1 + join => last.1 = last.1.max(b),
            _ => joined.push((a, b)),
        }
    }
    joined
}

/// Stretches of sound between silences (Audacity's Label Sounds): 10 ms
/// blocks whose loudest sample is above `threshold_db`, joined across
/// silences shorter than `min_silence` s, kept if at least `min_sound` s.
pub fn label_sounds(buffer: &AudioBuffer, threshold_db: f64, min_silence: f64, min_sound: f64) -> Vec<(usize, usize)> {
    let rate = f64::from(buffer.sample_rate);
    let block = ((0.01 * rate) as usize).max(1);
    let n = buffer.frames();
    let limit = 10f64.powf(threshold_db / 20.0) as f32;
    let gap = (min_silence.max(0.0) * rate) as usize;
    let mut sounds: Vec<(usize, usize)> = Vec::new();
    for b in 0..n.div_ceil(block) {
        let (a, e) = (b * block, ((b + 1) * block).min(n));
        if !buffer.channels.iter().any(|c| c[a..e].iter().any(|s| s.abs() > limit)) {
            continue;
        }
        match sounds.last_mut() {
            Some(last) if a <= last.1 + gap => last.1 = e,
            _ => sounds.push((a, e)),
        }
    }
    sounds.retain(|(a, b)| (b - a) as f64 >= min_sound * rate);
    sounds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, amp: f32, rate: u32, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin() as f32 * amp).collect()
    }

    #[test]
    fn the_spectrum_finds_a_tone_at_its_level() {
        let rate = 44_100;
        let plot = plot_spectrum(&AudioBuffer::new(rate, vec![tone(1000.0, 0.5, rate, 44_100)]), 4096).unwrap();
        assert!((plot.peak_hz - 1000.0).abs() < 3.0, "{}", plot.peak_hz);
        let top = plot.db.iter().copied().fold(f32::MIN, f32::max);
        assert!((top + 6.0).abs() < 1.6, "a half-scale sine near −6 dB: {top}");
        assert!(plot_spectrum(&AudioBuffer::new(rate, vec![vec![0.0; 100]]), 4096).is_err());
    }

    #[test]
    fn rms_clipping_and_sounds_are_found() {
        let rate = 1000;
        assert!((rms_db(&AudioBuffer::new(rate, vec![vec![0.1; 1000]])) + 20.0).abs() < 1e-5);
        let mut x = vec![0.0_f32; 5000];
        // Two clipped stretches 5 ms apart (joined), one far away, one too short to count.
        x[100..110].fill(1.0);
        x[115..120].fill(-1.0);
        x[3000..3008].fill(1.0);
        x[4000..4001].fill(1.0);
        let found = find_clipping(&AudioBuffer::new(rate, vec![x]), 0.99, 3);
        assert_eq!(found, vec![(100, 120), (3000, 3008)]);
        // Sound 0.5–1.2 s with a 0.2 s pause in it, and 3–3.05 s (too short).
        let mut y = vec![0.0_f32; 4000];
        y[500..800].fill(0.5);
        y[1000..1200].fill(0.5);
        y[3000..3050].fill(0.5);
        let sounds = label_sounds(&AudioBuffer::new(rate, vec![y]), -30.0, 0.5, 0.1);
        assert_eq!(sounds, vec![(500, 1200)]);
    }
}
