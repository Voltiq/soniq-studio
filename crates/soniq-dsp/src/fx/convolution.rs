//! A convolution reverb: the sound played through the recorded "echo" of a
//! space, an impulse response. The built-in responses are synthesised
//! (decaying noise, darker as it fades, with early reflections for rooms),
//! so they ship with the app and any length is possible; an audio file
//! loaded into the device is used as the response instead.
//!
//! Uniformly partitioned convolution: the response is cut into blocks of
//! `BLOCK` samples, each held as a spectrum; every `BLOCK` samples of input
//! are transformed once and multiplied with all of them. The wet signal is
//! one block late (about 11 ms at 48 kHz, heard as a little pre-delay); the
//! dry signal is not delayed. Everything is allocated when it is built.

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use soniq_io::AudioBuffer;

use super::{DelayLine, Smooth, choice};
use crate::Params;
use crate::common::{Noise, Svf, SvfCoef};

const BLOCK: usize = 512;
const SIZE: usize = 2 * BLOCK;
/// Longest response, in seconds.
const LONGEST: f32 = 8.0;

/// (decay to −60 dB in seconds, early reflections, brightness 0–1) per space.
const SPACES: [(f32, bool, f32); 5] = [(0.7, true, 0.6), (1.3, true, 0.5), (2.4, true, 0.45), (1.8, false, 0.9), (4.5, true, 0.35)];

pub struct Convolution {
    sr: f32,
    /// What the response was made from (space, length, the loaded file), so a
    /// rebuilt reverb with the same one keeps its tail.
    made: (usize, i64, usize),
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    /// The response's spectra, left and right, one per block.
    response: [Vec<Vec<Complex32>>; 2],
    /// The latest input spectra, newest at `head`.
    history: Vec<Vec<Complex32>>,
    head: usize,
    /// The last two blocks of input (mono), and how much of the newest is filled.
    input: Vec<f32>,
    filled: usize,
    /// Wet output of the last block, left and right.
    output: [Vec<f32>; 2],
    work: Vec<Complex32>,
    sum: Vec<Complex32>,
    predelay: [DelayLine; 2],
    delay: f32,
    tone: [Svf; 4],
    lowcut: SvfCoef,
    highcut: SvfCoef,
    width: f32,
    mix: Smooth,
}

/// The impulse response of space `space`, `scale` times as long, left and right.
fn impulse(space: usize, scale: f32, sr: f32) -> [Vec<f32>; 2] {
    let (decay, early, bright) = SPACES[space.min(SPACES.len() - 1)];
    let decay = (decay * scale).clamp(0.1, LONGEST / 1.2);
    let len = ((decay * 1.2 * sr) as usize).max(BLOCK);
    let mut out = [vec![0.0; len], vec![0.0; len]];
    for (ch, ir) in out.iter_mut().enumerate() {
        let mut noise = Noise(0x9e37_79b9 ^ (space as u32 * 7919 + ch as u32 * 104_729 + 1));
        // Air soaks up the highs first: a one-pole low-pass that closes as the tail fades.
        let mut lp = 0.0f32;
        for (i, s) in ir.iter_mut().enumerate() {
            let t = i as f32 / sr;
            let level = (-6.91 * t / decay).exp();
            let cutoff = bright * (1.0 - 0.8 * (t / decay).min(1.0));
            lp += (noise.next() - lp) * cutoff.clamp(0.02, 1.0);
            // A short fade-in so the onset does not click.
            *s = lp * level * (t / 0.005).min(1.0);
        }
        if early {
            // A handful of first reflections off the walls.
            for k in 1..=8 {
                let at = ((0.007 * k as f32 + 0.003 * ((k * (ch + 2)) % 5) as f32) * sr * (0.6 + 0.4 * scale.min(2.0))) as usize;
                if let Some(s) = ir.get_mut(at) {
                    *s += 0.6 * (-(k as f32) / 4.0).exp() * if (k + ch) % 2 == 0 { 1.0 } else { -1.0 };
                }
            }
        }
    }
    // The same loudness from every space: unit energy.
    let energy: f32 = out.iter().flatten().map(|s| s * s).sum::<f32>() / 2.0;
    let norm = 1.0 / energy.sqrt().max(1e-9);
    for s in out.iter_mut().flatten() {
        *s *= norm;
    }
    out
}

/// A loaded file as the response: its first `scale` (up to all of it, and
/// `LONGEST` seconds), from where its sound starts, at the loudness of the
/// built-in spaces. A mono file gives the same response on both sides.
fn loaded(buffer: &AudioBuffer, scale: f32, sr: f32) -> Option<[Vec<f32>; 2]> {
    let left = buffer.channels.first()?;
    let right = buffer.channels.get(1).unwrap_or(left);
    let frames = left.len().min(right.len());
    let top = left[..frames].iter().chain(&right[..frames]).fold(0.0f32, |p, s| p.max(s.abs()));
    if top <= 1e-6 {
        return None;
    }
    // Silence before the first sound would only be a pre-delay nobody set.
    let start = (0..frames).find(|&i| left[i].abs().max(right[i].abs()) > top * 0.001).unwrap_or(0);
    let whole = (frames - start).min((LONGEST * sr) as usize);
    let len = ((whole as f32 * scale.min(1.0)) as usize).clamp(BLOCK.min(whole), whole);
    // A response cut short fades out over its last tenth instead of stopping dead.
    let fade = if len < frames - start { (len / 10).max(1) } else { 0 };
    let mut out = [left, right].map(|side| {
        let mut ir = vec![0.0; len.max(BLOCK)];
        for (i, s) in ir.iter_mut().take(len).enumerate() {
            let tail = len - i;
            *s = side[start + i] * if tail <= fade { tail as f32 / fade as f32 } else { 1.0 };
        }
        ir
    });
    let energy: f32 = out.iter().flatten().map(|s| s * s).sum::<f32>() / 2.0;
    let norm = 1.0 / energy.sqrt().max(1e-9);
    for s in out.iter_mut().flatten() {
        *s *= norm;
    }
    Some(out)
}

impl Convolution {
    /// `file` is the device's own impulse response, at the engine's rate; without one, the space chosen plays.
    pub fn new(sr: f32, p: &dyn Params, file: Option<&Arc<AudioBuffer>>) -> Self {
        let space = choice(p, "space", 2.0);
        let scale = (p.or("length", 100.0).clamp(25.0, 200.0) / 100.0) as f32;
        let own = file.and_then(|f| Some((loaded(f, scale, sr)?, Arc::as_ptr(f) as usize)));
        let (ir, source) = match own {
            Some((ir, id)) => (ir, id),
            None => (impulse(space, scale, sr), 0),
        };
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(SIZE);
        let ifft = planner.plan_fft_inverse(SIZE);
        let scratch_len = fft.get_inplace_scratch_len().max(ifft.get_inplace_scratch_len());
        let mut scratch = vec![Complex32::default(); scratch_len];
        let parts = ir[0].len().div_ceil(BLOCK);
        let response = ir.map(|side| {
            (0..parts)
                .map(|k| {
                    // Each block zero-padded to the transform size.
                    let mut spec = vec![Complex32::default(); SIZE];
                    for (i, s) in side[k * BLOCK..((k + 1) * BLOCK).min(side.len())].iter().enumerate() {
                        spec[i] = Complex32::new(*s, 0.0);
                    }
                    fft.process_with_scratch(&mut spec, &mut scratch);
                    spec
                })
                .collect()
        });
        let mut reverb = Self {
            sr,
            // With a file, the space chosen makes no difference.
            made: (if source == 0 { space } else { 0 }, (scale * 1000.0) as i64, source),
            fft,
            ifft,
            scratch,
            response,
            history: vec![vec![Complex32::default(); SIZE]; parts],
            head: 0,
            input: vec![0.0; SIZE],
            filled: 0,
            output: [vec![0.0; BLOCK], vec![0.0; BLOCK]],
            work: vec![Complex32::default(); SIZE],
            sum: vec![Complex32::default(); SIZE],
            predelay: [DelayLine::new(0.25, sr), DelayLine::new(0.25, sr)],
            delay: 0.0,
            tone: [Svf::default(); 4],
            lowcut: SvfCoef::default(),
            highcut: SvfCoef::default(),
            width: 1.0,
            mix: Smooth::new((p.or("mix", 30.0) / 100.0).clamp(0.0, 1.0) as f32, sr),
        };
        reverb.configure(p);
        reverb
    }

    /// Pre-delay, tone, width and mix move freely; the space and its length
    /// need a rebuild (a new response), which the engine does on a change.
    pub fn configure(&mut self, p: &dyn Params) {
        self.delay = (p.or("predelay", 0.0).clamp(0.0, 200.0) * 0.001) as f32 * self.sr;
        self.lowcut = SvfCoef::new(p.or("lowcut", 100.0).clamp(20.0, 2000.0) as f32, 0.2, self.sr);
        self.highcut = SvfCoef::new(p.or("highcut", 9000.0).clamp(1000.0, 20_000.0) as f32, 0.2, self.sr);
        self.width = (p.or("width", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
        self.mix.set((p.or("mix", 30.0) / 100.0).clamp(0.0, 1.0) as f32);
    }

    /// Convolves the block of input just completed and leaves its wet output in `output`.
    fn convolve(&mut self) {
        let parts = self.history.len();
        self.head = (self.head + 1) % parts;
        let spec = &mut self.history[self.head];
        for (s, x) in spec.iter_mut().zip(&self.input) {
            *s = Complex32::new(*x, 0.0);
        }
        self.fft.process_with_scratch(spec, &mut self.scratch);
        for side in 0..2 {
            self.sum.fill(Complex32::default());
            for (k, part) in self.response[side].iter().enumerate() {
                let x = &self.history[(self.head + parts - k) % parts];
                // Real signals: the upper half mirrors the lower one.
                for i in 0..=SIZE / 2 {
                    self.sum[i] += x[i] * part[i];
                }
            }
            for i in 1..SIZE / 2 {
                self.sum[SIZE - i] = self.sum[i].conj();
            }
            self.work.copy_from_slice(&self.sum);
            self.ifft.process_with_scratch(&mut self.work, &mut self.scratch);
            // Overlap-save: the second half is the new block.
            for (o, w) in self.output[side].iter_mut().zip(&self.work[BLOCK..]) {
                *o = w.re / SIZE as f32;
            }
        }
        // The newest block becomes the older half of the next transform.
        self.input.copy_within(BLOCK.., 0);
        self.filled = 0;
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.input[BLOCK + self.filled] = 0.5 * (*l + *r);
            let (wl, wr) = (self.output[0][self.filled], self.output[1][self.filled]);
            self.filled += 1;
            if self.filled == BLOCK {
                self.convolve();
            }
            // Pre-delay, then the tone: low cut and high cut on the wet sound only.
            self.predelay[0].push(wl);
            self.predelay[1].push(wr);
            let (mut a, mut b) = if self.delay >= 1.0 { (self.predelay[0].read(self.delay), self.predelay[1].read(self.delay)) } else { (wl, wr) };
            a = self.tone[0].tick(&self.lowcut, a).2;
            b = self.tone[1].tick(&self.lowcut, b).2;
            a = self.tone[2].tick(&self.highcut, a).0;
            b = self.tone[3].tick(&self.highcut, b).0;
            // Width: from mono (0) to the full stereo response.
            let mid = 0.5 * (a + b);
            let (a, b) = (mid + (a - mid) * self.width, mid + (b - mid) * self.width);
            let m = self.mix.next();
            *l = *l * (1.0 - m) + a * m;
            *r = *r * (1.0 - m) + b * m;
        }
    }

    /// The same space keeps its tail ringing through a rebuild.
    pub fn adopt(&mut self, old: &mut Convolution) {
        if self.made == old.made && self.history.len() == old.history.len() {
            std::mem::swap(&mut self.history, &mut old.history);
            std::mem::swap(&mut self.input, &mut old.input);
            std::mem::swap(&mut self.output, &mut old.output);
            (self.head, self.filled) = (old.head, old.filled);
        }
        for (new, old) in self.predelay.iter_mut().zip(old.predelay.iter_mut()) {
            new.adopt(old);
        }
        self.tone = old.tone;
        self.mix.adopt(&old.mix);
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;
    use super::BLOCK;

    #[test]
    fn a_click_leaves_a_decaying_tail_one_block_late() {
        let mut reverb = AudioFx::build("convolution", SR, &params(&[("space", 0.0), ("mix", 100.0), ("lowcut", 20.0), ("highcut", 20_000.0)])).unwrap();
        let mut click = vec![0.0f32; 48_000];
        click[0] = 1.0;
        let (l, r) = run(&mut reverb, &click);
        assert!(peak(&l[..BLOCK - 1]) < 1e-6, "nothing before the first block is done");
        let early = rms(&l[BLOCK..BLOCK + 4800]);
        let late = rms(&l[BLOCK + 24_000..BLOCK + 28_800]);
        assert!(early > 1e-3 && late < early * 0.05, "{early} {late}");
        // Left and right differ: a wide reverb.
        assert!(l[BLOCK + 2000..BLOCK + 2100].iter().zip(&r[BLOCK + 2000..BLOCK + 2100]).any(|(a, b)| (a - b).abs() > 1e-4));
        // Without wet, the sound passes untouched.
        let mut dry = AudioFx::build("convolution", SR, &params(&[("mix", 0.0)])).unwrap();
        let tone = sine(300.0, 0.5, 9600);
        let (l, _) = run(&mut dry, &tone);
        assert!(l.iter().zip(&tone).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn a_loaded_file_is_the_response() {
        use std::sync::Arc;

        use soniq_io::AudioBuffer;

        use crate::SampleSlot;

        // Silence, then a response of two clicks 1000 samples apart, the second one half as loud and inverted.
        let mut ir = vec![0.0f32; 4000];
        ir[300] = 0.8;
        ir[1300] = -0.4;
        let slot = SampleSlot { key: 0, buffer: Arc::new(AudioBuffer::new(SR, vec![ir])), onsets: None };
        let p = params(&[("mix", 100.0), ("lowcut", 20.0), ("highcut", 20_000.0), ("width", 100.0)]);
        let mut reverb = AudioFx::build_with("convolution", SR, &p, std::slice::from_ref(&slot)).unwrap();
        let mut click = vec![0.0f32; 9600];
        click[0] = 1.0;
        let (l, r) = run(&mut reverb, &click);
        // One block late, with the leading silence gone: the clicks land at BLOCK and BLOCK + 1000.
        let first = (0..l.len()).max_by(|a, b| l[*a].abs().total_cmp(&l[*b].abs())).unwrap();
        assert!((first as i64 - BLOCK as i64).abs() <= 3, "{first}");
        let second = l[BLOCK + 1000 - 3..BLOCK + 1000 + 4].iter().fold(0.0f32, |m, s| if s.abs() > m.abs() { *s } else { m });
        assert!(l[first] > 0.0 && second < 0.0 && (second / l[first] + 0.5).abs() < 0.15, "{} {second}", l[first]);
        // Nothing between the clicks, and a mono file plays the same on both sides.
        assert!(peak(&l[BLOCK + 100..BLOCK + 900]) < l[first] * 0.1);
        assert!(l.iter().zip(&r).all(|(a, b)| (a - b).abs() < 1e-5));
    }
}
