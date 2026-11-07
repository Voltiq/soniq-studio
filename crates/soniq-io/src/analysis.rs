//! Live analysis of the mix as it plays: loudness as EBU R128 meters show
//! it (momentary, short-term, integrated, loudness range), true peak, RMS,
//! stereo correlation and a spectrum. Fed stereo audio in any block sizes;
//! read whenever the screen wants a new frame.

use std::collections::VecDeque;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use serde::Serialize;

use crate::loudness::{Biquad, k_weighting, to_lufs};

/// Spectrum bands: a sixth of an octave each, centred on `20 · 2^(k/6)` Hz.
pub const SPECTRUM_BANDS: usize = 61;
/// The spectrum's floor, in dB.
pub const SPECTRUM_FLOOR: f32 = -100.0;
const FFT_LEN: usize = 8192;
/// How fast a band falls back after a peak, in dB per reading (about 30 a second).
const SPECTRUM_FALL: f32 = 1.2;
/// Loudness histogram: 0.1 LU steps from the −70 LUFS gate to +10.
const HIST_FLOOR: f64 = -70.0;
const HIST_BINS: usize = 800;
/// True peak: 4× oversampling with 12 taps per phase (ITU-R BS.1770-4, annex 2).
const PHASES: usize = 4;
const TAPS: usize = 12;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    /// LUFS over the last 400 ms and 3 s, and since the last reset;
    /// `None` below the −70 LUFS gate.
    pub momentary: Option<f64>,
    pub short_term: Option<f64>,
    pub integrated: Option<f64>,
    /// Loudness range since the last reset, in LU (EBU Tech 3342).
    pub range: Option<f64>,
    /// Highest true peak since the last reset, dBTP.
    pub true_peak: Option<f64>,
    /// Each channel's true peak since the last reading, dBTP.
    pub peak: [Option<f64>; 2],
    /// Each channel's RMS over the last 300 ms, dBFS.
    pub rms: [Option<f64>; 2],
    /// From −1 (out of phase) through 0 (unrelated) to +1 (mono), over the
    /// last 300 ms; `None` in silence.
    pub correlation: Option<f64>,
    /// dB in each of the [`SPECTRUM_BANDS`] bands, a full-scale sine reading 0.
    pub spectrum: Vec<f32>,
}

/// Loudness values in 0.1 LU steps, with the power behind each step, so
/// gating and percentiles never revisit the blocks themselves.
struct Histogram {
    count: Vec<u64>,
    power: Vec<f64>,
}

impl Histogram {
    fn new() -> Self {
        Self { count: vec![0; HIST_BINS], power: vec![0.0; HIST_BINS] }
    }

    fn add(&mut self, power: f64) {
        let lufs = to_lufs(power);
        if lufs.is_nan() || lufs <= HIST_FLOOR {
            return;
        }
        let bin = (((lufs - HIST_FLOOR) * 10.0) as usize).min(HIST_BINS - 1);
        self.count[bin] += 1;
        self.power[bin] += power;
    }

    fn clear(&mut self) {
        self.count.fill(0);
        self.power.fill(0.0);
    }

    /// The first bin above `gate_lu` below the mean of everything counted.
    fn gated_from(&self, gate_lu: f64) -> Option<usize> {
        let (n, sum) = self.count.iter().zip(&self.power).fold((0, 0.0), |(n, s), (c, p)| (n + c, s + p));
        if n == 0 {
            return None;
        }
        let gate = to_lufs(sum / n as f64) - gate_lu;
        Some((((gate - HIST_FLOOR) * 10.0).ceil().max(0.0) as usize).min(HIST_BINS))
    }

    /// Integrated loudness: the mean power above the relative gate (−10 LU).
    fn integrated(&self) -> Option<f64> {
        let from = self.gated_from(10.0)?;
        let (n, sum) = self.count[from..].iter().zip(&self.power[from..]).fold((0, 0.0), |(n, s), (c, p)| (n + c, s + p));
        (n > 0).then(|| to_lufs(sum / n as f64))
    }

    /// Loudness range: from the 10th to the 95th percentile above the relative gate (−20 LU).
    fn range(&self) -> Option<f64> {
        let from = self.gated_from(20.0)?;
        let counts = &self.count[from..];
        let n: u64 = counts.iter().sum();
        if n < 2 {
            return None;
        }
        let percentile = |p: f64| {
            let target = (p * (n - 1) as f64).round() as u64;
            let mut seen = 0;
            counts.iter().position(|c| {
                seen += c;
                seen > target
            })
        };
        let (low, high) = (percentile(0.10)?, percentile(0.95)?);
        Some((high - low) as f64 / 10.0)
    }
}

pub struct Analyzer {
    rate: u32,
    /// K-weighting per channel: shelf and high-pass, each with its state.
    k: [[(Biquad, [f64; 4]); 2]; 2],
    /// Samples per 100 ms step; everything is gathered in such steps.
    step: usize,
    filled: usize,
    k_power: f64,
    squares: [f64; 2],
    cross: f64,
    /// The last 30 steps' K-weighted power (3 s), and the last 3 steps'
    /// squares and cross products per channel (300 ms).
    loud: VecDeque<f64>,
    level: VecDeque<([f64; 2], f64)>,
    blocks: Histogram,
    short: Histogram,
    taps: [[f64; TAPS]; PHASES],
    history: [[f32; TAPS]; 2],
    peak: [f32; 2],
    max_peak: f32,
    /// The latest mono samples, for the spectrum.
    recent: VecDeque<f32>,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    bands: Vec<f32>,
}

impl Analyzer {
    pub fn new(rate: u32) -> Self {
        let (shelf, highpass) = k_weighting(f64::from(rate));
        // A windowed sinc interpolating three new samples between each two.
        let len = PHASES * TAPS;
        let centre = (len - 1) as f64 / 2.0;
        let mut taps = [[0.0; TAPS]; PHASES];
        for (n, tap) in (0..len).map(|n| {
            let x = (n as f64 - centre) / PHASES as f64;
            let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
            let blackman = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (len - 1) as f64).cos() + 0.08 * (4.0 * std::f64::consts::PI * n as f64 / (len - 1) as f64).cos();
            (n, sinc * blackman)
        }) {
            taps[n % PHASES][n / PHASES] = tap;
        }
        // Each phase passes a steady level unchanged.
        for phase in &mut taps {
            let sum: f64 = phase.iter().sum();
            phase.iter_mut().for_each(|t| *t /= sum);
        }
        let window = (0..FFT_LEN).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / FFT_LEN as f64).cos()) as f32).collect();
        Self {
            rate,
            k: [[(shelf, [0.0; 4]), (highpass, [0.0; 4])]; 2],
            step: (rate as usize / 10).max(1),
            filled: 0,
            k_power: 0.0,
            squares: [0.0; 2],
            cross: 0.0,
            loud: VecDeque::with_capacity(31),
            level: VecDeque::with_capacity(4),
            blocks: Histogram::new(),
            short: Histogram::new(),
            taps,
            history: [[0.0; TAPS]; 2],
            peak: [0.0; 2],
            max_peak: 0.0,
            recent: VecDeque::with_capacity(FFT_LEN + 1),
            fft: FftPlanner::new().plan_fft_forward(FFT_LEN),
            window,
            bands: vec![SPECTRUM_FLOOR; SPECTRUM_BANDS],
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Starts the integrated loudness, the range and the highest peak over.
    pub fn reset(&mut self) {
        self.blocks.clear();
        self.short.clear();
        self.max_peak = 0.0;
    }

    /// Takes in interleaved stereo audio.
    pub fn push_interleaved(&mut self, samples: &[f32]) {
        for [left, right] in samples.as_chunks::<2>().0 {
            self.push(*left, *right);
        }
    }

    fn push(&mut self, left: f32, right: f32) {
        let mut k_power = 0.0;
        for (ch, x) in [left, right].into_iter().enumerate() {
            let mut y = f64::from(x);
            for (filter, state) in &mut self.k[ch] {
                y = filter.step(state, y);
            }
            k_power += y * y;
            self.true_peak(ch, x);
        }
        self.k_power += k_power;
        self.squares[0] += f64::from(left) * f64::from(left);
        self.squares[1] += f64::from(right) * f64::from(right);
        self.cross += f64::from(left) * f64::from(right);
        self.recent.push_back((left + right) * 0.5);
        if self.recent.len() > FFT_LEN {
            self.recent.pop_front();
        }
        self.filled += 1;
        if self.filled == self.step {
            self.end_step();
        }
    }

    fn true_peak(&mut self, ch: usize, x: f32) {
        let history = &mut self.history[ch];
        history.copy_within(0..TAPS - 1, 1);
        history[0] = x;
        let mut peak = x.abs();
        for phase in &self.taps {
            let y: f64 = phase.iter().zip(history.iter()).map(|(t, s)| t * f64::from(*s)).sum();
            peak = peak.max(y.abs() as f32);
        }
        self.peak[ch] = self.peak[ch].max(peak);
        self.max_peak = self.max_peak.max(peak);
    }

    fn end_step(&mut self) {
        let n = self.filled as f64;
        self.loud.push_back(self.k_power / n);
        if self.loud.len() > 30 {
            self.loud.pop_front();
        }
        self.level.push_back((self.squares.map(|s| s / n), self.cross / n));
        if self.level.len() > 3 {
            self.level.pop_front();
        }
        // Gating blocks: 400 ms every 100 ms (75 % overlap); short-term
        // values for the range: 3 s every 100 ms.
        if let Some(power) = self.mean_loud(4) {
            self.blocks.add(power);
        }
        if let Some(power) = self.mean_loud(30) {
            self.short.add(power);
        }
        (self.filled, self.k_power, self.squares, self.cross) = (0, 0.0, [0.0; 2], 0.0);
    }

    /// Mean K-weighted power of the last `steps` steps, once there are that many.
    fn mean_loud(&self, steps: usize) -> Option<f64> {
        (self.loud.len() >= steps).then(|| self.loud.iter().rev().take(steps).sum::<f64>() / steps as f64)
    }

    /// What the meters show now. Clears the per-reading peaks.
    pub fn reading(&mut self) -> Reading {
        let gated = |power: Option<f64>| power.map(to_lufs).filter(|l| *l > HIST_FLOOR);
        let db = |v: f64| (v > 1e-10).then(|| 10.0 * v.log10());
        let peak_db = |p: f32| (p > 1e-5).then(|| 20.0 * f64::from(p).log10());
        let steps = self.level.len().max(1) as f64;
        let (squares, cross) = self.level.iter().fold(([0.0; 2], 0.0), |(s, c), (sq, x)| ([s[0] + sq[0], s[1] + sq[1]], c + x));
        let (ms, cross) = (squares.map(|s| s / steps), cross / steps);
        let correlation = (ms[0] > 1e-10 && ms[1] > 1e-10).then(|| (cross / (ms[0] * ms[1]).sqrt()).clamp(-1.0, 1.0));
        let reading = Reading {
            momentary: gated(self.mean_loud(4)),
            short_term: gated(self.mean_loud(30)),
            integrated: self.blocks.integrated(),
            range: self.short.range(),
            true_peak: peak_db(self.max_peak),
            peak: self.peak.map(peak_db),
            rms: ms.map(db),
            correlation,
            spectrum: self.spectrum(),
        };
        self.peak = [0.0; 2];
        reading
    }

    fn spectrum(&mut self) -> Vec<f32> {
        let mut buf: Vec<Complex<f32>> = vec![Complex::default(); FFT_LEN];
        let start = FFT_LEN - self.recent.len();
        for ((slot, s), w) in buf[start..].iter_mut().zip(&self.recent).zip(&self.window[start..]) {
            *slot = Complex::new(s * w, 0.0);
        }
        self.fft.process(&mut buf);
        // A full-scale sine reads 0 dB: the Hann window halves the amplitude.
        let scale = 4.0 / FFT_LEN as f32;
        let power: Vec<f32> = buf[..FFT_LEN / 2].iter().map(|c| (c.norm() * scale).powi(2)).collect();
        let bin_hz = self.rate as f32 / FFT_LEN as f32;
        for (k, band) in self.bands.iter_mut().enumerate() {
            let centre = 20.0 * 2f32.powf(k as f32 / 6.0);
            let (lo, hi) = (centre * 2f32.powf(-1.0 / 12.0), centre * 2f32.powf(1.0 / 12.0));
            let (first, last) = ((lo / bin_hz).ceil() as usize, ((hi / bin_hz).floor() as usize).min(power.len() - 1));
            // Narrow low bands may hold no bin: the nearest one stands in.
            let p = if first <= last { power[first..=last].iter().copied().fold(0.0, f32::max) } else { power[((centre / bin_hz).round() as usize).min(power.len() - 1)] };
            let now = if p > 0.0 { (10.0 * p.log10()).max(SPECTRUM_FLOOR) } else { SPECTRUM_FLOOR };
            *band = now.max(*band - SPECTRUM_FALL);
        }
        self.bands.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn feed(a: &mut Analyzer, seconds: f64, f: impl Fn(f64) -> (f32, f32)) {
        let n = (f64::from(a.rate) * seconds) as usize;
        let samples: Vec<f32> = (0..n).flat_map(|i| {
            let (l, r) = f(i as f64 / f64::from(a.rate));
            [l, r]
        }).collect();
        a.push_interleaved(&samples);
    }

    fn sine(freq: f64, dbfs: f64) -> impl Fn(f64) -> (f32, f32) {
        let amp = 10f64.powf(dbfs / 20.0);
        move |t| {
            let v = (amp * (2.0 * PI * freq * t).sin()) as f32;
            (v, v)
        }
    }

    #[test]
    fn the_reference_tone_reads_minus_23_everywhere() {
        // EBU Tech 3341: a stereo 1 kHz sine at −23 dBFS is −23 LUFS momentary, short-term and integrated.
        let mut a = Analyzer::new(48_000);
        feed(&mut a, 10.0, sine(1000.0, -23.0));
        let r = a.reading();
        for (what, lufs) in [("momentary", r.momentary), ("short-term", r.short_term), ("integrated", r.integrated)] {
            let lufs = lufs.unwrap();
            assert!((lufs + 23.0).abs() < 0.1, "{what}: {lufs}");
        }
        assert!(r.range.unwrap() < 0.2, "a steady tone has no range: {:?}", r.range);
        assert!((r.rms[0].unwrap() + 26.0).abs() < 0.1, "RMS of a sine is 3 dB under its peak: {:?}", r.rms);
        assert!((r.correlation.unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn integrated_loudness_gates_out_silence_and_resets() {
        // Live, it agrees with the export's measurement of the same audio.
        let tone = sine(1000.0, -23.0);
        let n = 48_000 * 10;
        let left: Vec<f32> = (0..n).map(|i| if i < n / 2 { tone(i as f64 / 48_000.0).0 } else { 0.0 }).collect();
        let offline = crate::integrated_loudness(&crate::AudioBuffer::new(48_000, vec![left.clone(), left.clone()])).unwrap();
        let mut a = Analyzer::new(48_000);
        a.push_interleaved(&left.iter().flat_map(|v| [*v, *v]).collect::<Vec<f32>>());
        let r = a.reading();
        assert!((r.integrated.unwrap() - offline).abs() < 0.05, "{:?} live, {offline} offline", r.integrated);
        assert!((offline + 23.0).abs() < 0.2, "silence gated out: {offline}");
        assert_eq!(r.momentary, None, "silence now");
        a.reset();
        assert_eq!(a.reading().integrated, None);
    }

    #[test]
    fn loudness_range_spans_the_quiet_and_loud_parts() {
        // EBU Tech 3342 case 1: 20 s at −20 dBFS, then 20 s at −30: LRA 10 LU.
        let mut a = Analyzer::new(48_000);
        feed(&mut a, 20.0, sine(1000.0, -20.0));
        feed(&mut a, 20.0, sine(1000.0, -30.0));
        let range = a.reading().range.unwrap();
        assert!((range - 10.0).abs() < 1.0, "{range}");
    }

    #[test]
    fn true_peak_finds_the_peak_between_samples() {
        // A sine at a quarter of the rate, 45° off: every sample is at 0.707,
        // the wave itself reaches 1.0.
        let mut a = Analyzer::new(48_000);
        feed(&mut a, 0.5, |t| {
            let v = (2.0 * PI * 12_000.0 * t + PI / 4.0).sin() as f32;
            (v, v)
        });
        let r = a.reading();
        let tp = r.true_peak.unwrap();
        assert!(tp > -0.6 && tp < 0.3, "true peak {tp} dBTP, sample peak would read −3");
    }

    #[test]
    fn correlation_tells_mono_from_out_of_phase_and_unrelated() {
        let mut a = Analyzer::new(48_000);
        feed(&mut a, 0.5, |t| {
            let v = (2.0 * PI * 440.0 * t).sin() as f32 * 0.5;
            (v, -v)
        });
        assert!((a.reading().correlation.unwrap() + 1.0).abs() < 1e-6);
        feed(&mut a, 0.5, |t| ((2.0 * PI * 440.0 * t).sin() as f32 * 0.5, (2.0 * PI * 1250.0 * t).sin() as f32 * 0.5));
        assert!(a.reading().correlation.unwrap().abs() < 0.05);
    }

    #[test]
    fn the_spectrum_peaks_in_the_tones_band() {
        let mut a = Analyzer::new(48_000);
        feed(&mut a, 0.5, sine(1000.0, -6.0));
        let s = a.reading().spectrum;
        let band = (6.0 * (1000.0f64 / 20.0).log2()).round() as usize;
        let loudest = s.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).unwrap().0;
        assert_eq!(loudest, band);
        assert!((s[band] + 6.0).abs() < 1.0, "a −6 dBFS sine reads about −6 dB: {}", s[band]);
        assert!(s[band - 12] < -60.0, "an octave below is quiet: {}", s[band - 12]);
    }
}
