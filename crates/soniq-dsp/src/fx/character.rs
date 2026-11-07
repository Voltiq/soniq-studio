//! Movement and colour: auto pan (or tremolo), redux (fewer bits and a
//! lower sample rate) and a pitch shifter.

use super::{DelayLine, Smooth, choice};
use crate::common::{db_gain, sin_turns};
use crate::{FxContext, Params};

fn mix_of(p: &dyn Params, default: f64) -> f32 {
    (p.or("mix", default) / 100.0).clamp(0.0, 1.0) as f32
}

/// Sync choices in beats (Free, then 1/16 to 2 bars of 4/4).
const SYNC_BEATS: [f64; 7] = [0.0, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];

/// Moves the sound between left and right with an LFO; with no phase
/// between the sides it pulses the level instead (tremolo).
pub struct AutoPan {
    sr: f32,
    rate: f32,
    sync: f64,
    depth: f32,
    shape: usize,
    offset: f32,
    phase: f32,
}

impl AutoPan {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut pan = Self { sr, rate: 1.0, sync: 0.0, depth: 0.0, shape: 0, offset: 0.5, phase: 0.0 };
        pan.configure(p);
        pan
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.rate = p.or("rate", 1.0).clamp(0.01, 40.0) as f32;
        self.sync = SYNC_BEATS[choice(p, "sync", 0.0).min(SYNC_BEATS.len() - 1)];
        self.depth = (p.or("depth", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
        self.shape = choice(p, "shape", 0.0).min(2);
        self.offset = (p.or("phase", 180.0) / 360.0).rem_euclid(1.0) as f32;
    }

    fn wave(&self, t: f32) -> f32 {
        let t = t.rem_euclid(1.0);
        match self.shape {
            0 => sin_turns(t),
            1 => 1.0 - 4.0 * (t - 0.5).abs(),
            _ => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], ctx: &FxContext) {
        let n = left.len();
        // Synced, the LFO follows the song's beats while it plays.
        let (start, step) = if self.sync > 0.0 && ctx.playing {
            let per = (self.sync * ctx.samples_per_beat) as f32;
            (((ctx.beat / self.sync).fract()) as f32, 1.0 / per.max(1.0))
        } else if self.sync > 0.0 {
            (self.phase, 1.0 / (self.sync * ctx.samples_per_beat).max(1.0) as f32)
        } else {
            (self.phase, self.rate / self.sr)
        };
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let t = start + step * i as f32;
            let gl = 1.0 - self.depth * (0.5 - 0.5 * self.wave(t));
            let gr = 1.0 - self.depth * (0.5 - 0.5 * self.wave(t + self.offset));
            *l *= gl;
            *r *= gr;
        }
        self.phase = (start + step * n as f32).fract();
    }

    pub fn adopt(&mut self, old: &mut AutoPan) {
        self.phase = old.phase;
    }
}

/// Lo-fi: fewer bits (coarser steps) and a lower sample rate (each sample
/// held for a while), mixed with the clean sound.
pub struct Redux {
    steps: f32,
    hold: f32,
    mix: f32,
    counter: f32,
    held: [f32; 2],
}

impl Redux {
    pub fn new(_sr: f32, p: &dyn Params) -> Self {
        let mut redux = Self { steps: 0.0, hold: 1.0, mix: 1.0, counter: 0.0, held: [0.0; 2] };
        redux.configure(p);
        redux
    }

    pub fn configure(&mut self, p: &dyn Params) {
        let bits = p.or("bits", 16.0).clamp(1.0, 24.0);
        self.steps = 2f64.powf(bits - 1.0) as f32;
        self.hold = p.or("down", 1.0).clamp(1.0, 100.0) as f32;
        self.mix = mix_of(p, 100.0);
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.counter -= 1.0;
            if self.counter <= 0.0 {
                self.counter += self.hold;
                self.held = [(*l * self.steps).round() / self.steps, (*r * self.steps).round() / self.steps];
            }
            *l = self.held[0] * self.mix + *l * (1.0 - self.mix);
            *r = self.held[1] * self.mix + *r * (1.0 - self.mix);
        }
    }

    pub fn adopt(&mut self, old: &mut Redux) {
        (self.counter, self.held) = (old.counter.min(self.hold), old.held);
    }
}

/// Shifts the pitch and keeps the timing: two taps sweep through a short
/// delay at the speed the shift needs, each faded out while it jumps back.
pub struct PitchShifter {
    sr: f32,
    lines: [DelayLine; 2],
    /// How fast the taps move through the delay (1 − pitch ratio), and the window they move in.
    speed: f32,
    window: f32,
    pos: f32,
    mix: f32,
    output: Smooth,
}

impl PitchShifter {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let lines = [DelayLine::new(0.25, sr), DelayLine::new(0.25, sr)];
        let mut ps = Self { sr, lines, speed: 0.0, window: 0.0, pos: 0.0, mix: 1.0, output: Smooth::new(db_gain(p.or("gain", 0.0)), sr) };
        ps.configure(p);
        ps
    }

    pub fn configure(&mut self, p: &dyn Params) {
        let semis = p.or("pitch", 0.0).clamp(-24.0, 24.0) + p.or("fine", 0.0).clamp(-50.0, 50.0) / 100.0;
        self.speed = 1.0 - 2f64.powf(semis / 12.0) as f32;
        self.window = (p.or("window", 60.0).clamp(20.0, 120.0) * 0.001) as f32 * self.sr;
        self.pos = self.pos.min(self.window);
        self.mix = mix_of(p, 100.0);
        self.output.set(db_gain(p.or("gain", 0.0)));
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let w = self.window;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.lines[0].push(*l);
            self.lines[1].push(*r);
            self.pos = (self.pos + self.speed).rem_euclid(w);
            let other = (self.pos + 0.5 * w).rem_euclid(w);
            // Each tap is loudest mid-window, silent where it wraps.
            let ga = sin_turns(0.5 * self.pos / w);
            let gb = sin_turns(0.5 * other / w);
            let g = self.output.next();
            for (ch, x) in [l, r].into_iter().enumerate() {
                let wet = self.lines[ch].read(self.pos + 2.0) * ga + self.lines[ch].read(other + 2.0) * gb;
                *x = (wet * self.mix + *x * (1.0 - self.mix)) * g;
            }
        }
    }

    pub fn adopt(&mut self, old: &mut PitchShifter) {
        for (new, old) in self.lines.iter_mut().zip(old.lines.iter_mut()) {
            new.adopt(old);
        }
        self.pos = old.pos.min(self.window);
        self.output.adopt(&old.output);
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;

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
    fn auto_pan_moves_the_sound_side_to_side() {
        let mut pan = AudioFx::build("autopan", SR, &params(&[("rate", 2.0), ("depth", 100.0), ("phase", 180.0)])).unwrap();
        let ones = vec![1.0f32; 24_000];
        let (l, r) = run(&mut pan, &ones);
        // A quarter of a cycle in: left all the way up, right all the way down.
        assert!(l[6000] > 0.99 && r[6000] < 0.01, "{} {}", l[6000], r[6000]);
        assert!(l[18_000] < 0.01 && r[18_000] > 0.99);
    }

    #[test]
    fn redux_coarsens_steps_and_holds_samples() {
        let mut redux = AudioFx::build("redux", SR, &params(&[("bits", 2.0), ("down", 10.0)])).unwrap();
        let (l, _) = run(&mut redux, &sine(100.0, 0.9, 4800));
        // Two bits: only −1, −0.5, 0, 0.5 and 1.
        assert!(l.iter().all(|s| (s * 2.0 - (s * 2.0).round()).abs() < 1e-6));
        // Held for ten samples at a time.
        assert!(l[100..110].iter().all(|s| *s == l[100]) || l[101..111].iter().all(|s| *s == l[101]));
    }

    #[test]
    fn the_pitch_shifter_moves_a_tone_up_an_octave() {
        let mut ps = AudioFx::build("pitch", SR, &params(&[("pitch", 12.0)])).unwrap();
        let (l, _) = run(&mut ps, &sine(220.0, 0.5, 48_000));
        // Frame by frame (each tap's phase steps where it wraps, so one long
        // transform would smear it): the octave up carries the sound.
        let frames: Vec<&[f32]> = l[9600..].chunks(2048).filter(|f| f.len() == 2048).collect();
        let mean = |hz: f32| frames.iter().map(|f| tone_level(f, hz)).sum::<f32>() / frames.len() as f32;
        let (up, original) = (mean(440.0), mean(220.0));
        assert!(up > 0.12 && original < up * 0.1, "{up} {original}");
    }
}
