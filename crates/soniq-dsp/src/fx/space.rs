//! Space: an algorithmic reverb (Freeverb's comb and all-pass network, with
//! pre-delay, decay time and damping) and a delay (free or synced to the
//! tempo, stereo or ping-pong, with a low-pass in the feedback).

use std::f32::consts::TAU;

use super::{DelayLine, choice};
use crate::{FxContext, Params};

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
/// Extra samples on the right channel's lines, for a wide stereo image.
const SPREAD: usize = 23;
/// Room sizes run from 0.5× to this times Freeverb's tuning.
const MAX_SCALE: f32 = 1.6;

#[derive(Debug, Clone, Default)]
struct Comb {
    buf: Vec<f32>,
    len: usize,
    pos: usize,
    feedback: f32,
    store: f32,
}

impl Comb {
    fn tick(&mut self, x: f32, damp: f32) -> f32 {
        let y = self.buf[self.pos];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = x + self.store * self.feedback;
        self.pos = (self.pos + 1) % self.len;
        y
    }
}

#[derive(Debug, Clone, Default)]
struct Allpass {
    buf: Vec<f32>,
    len: usize,
    pos: usize,
}

impl Allpass {
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        self.buf[self.pos] = x + b * 0.5;
        self.pos = (self.pos + 1) % self.len;
        b - x
    }
}

pub struct Reverb {
    sr: f32,
    combs: [[Comb; 8]; 2],
    allpasses: [[Allpass; 4]; 2],
    predelay: DelayLine,
    predelay_samples: f32,
    damp: f32,
    mix: f32,
}

/// A line's tuning in samples for a room scale, on channel `ch`.
fn line_len(base: usize, ch: usize, rate: f32, scale: f32) -> usize {
    (((base + ch * SPREAD) as f32 * rate * scale) as usize).max(1)
}

impl Reverb {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let rate = sr / 44_100.0;
        // Lines are allocated at the largest room size, so any size fits later.
        let cap = |base: usize, ch: usize| line_len(base, ch, rate, MAX_SCALE) + 1;
        let mut verb = Self {
            sr,
            combs: std::array::from_fn(|ch| std::array::from_fn(|i| Comb { buf: vec![0.0; cap(COMBS[i], ch)], len: 1, ..Comb::default() })),
            allpasses: std::array::from_fn(|ch| std::array::from_fn(|i| Allpass { buf: vec![0.0; cap(ALLPASSES[i], ch)], len: 1, pos: 0 })),
            predelay: DelayLine::new(0.25, sr),
            predelay_samples: 0.0,
            damp: 0.0,
            mix: 0.0,
        };
        verb.configure(p);
        verb
    }

    pub fn configure(&mut self, p: &dyn Params) {
        let (sr, rate) = (self.sr, self.sr / 44_100.0);
        let size = (p.or("size", 55.0) / 100.0).clamp(0.0, 1.0) as f32;
        let scale = 0.5 + (MAX_SCALE - 0.5) * size;
        let decay = p.or("decay", 2.4).max(0.05) as f32;
        for (ch, combs) in self.combs.iter_mut().enumerate() {
            for (i, comb) in combs.iter_mut().enumerate() {
                comb.len = line_len(COMBS[i], ch, rate, scale).min(comb.buf.len());
                comb.pos %= comb.len;
                // Each comb loses 60 dB over the decay time.
                comb.feedback = 10f32.powf(-3.0 * comb.len as f32 / (decay * sr));
            }
        }
        for (ch, aps) in self.allpasses.iter_mut().enumerate() {
            for (i, ap) in aps.iter_mut().enumerate() {
                ap.len = line_len(ALLPASSES[i], ch, rate, scale).min(ap.buf.len());
                ap.pos %= ap.len;
            }
        }
        self.predelay_samples = (p.or("predelay", 20.0).clamp(0.0, 240.0) * 0.001) as f32 * sr;
        self.damp = (-TAU * p.or("damp", 6000.0).clamp(500.0, 20_000.0) as f32 / sr).exp();
        self.mix = (p.or("mix", 25.0) / 100.0).clamp(0.0, 1.0) as f32;
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let damp = self.damp;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.predelay.push((*l + *r) * 0.5);
            let input = self.predelay.read(self.predelay_samples) * 0.015;
            let mut wet = [0.0_f32; 2];
            for (ch, out) in wet.iter_mut().enumerate() {
                let mut sum = 0.0;
                for comb in &mut self.combs[ch] {
                    sum += comb.tick(input, damp);
                }
                for ap in &mut self.allpasses[ch] {
                    sum = ap.tick(sum);
                }
                *out = sum * 3.0;
            }
            *l = *l * (1.0 - self.mix) + wet[0] * self.mix;
            *r = *r * (1.0 - self.mix) + wet[1] * self.mix;
        }
    }

    /// The tail carries on: the lines are allocated at the largest size, so they always fit.
    pub fn adopt(&mut self, old: &mut Reverb) {
        for (new, old) in self.combs.iter_mut().flatten().zip(old.combs.iter_mut().flatten()) {
            if new.buf.len() == old.buf.len() {
                std::mem::swap(&mut new.buf, &mut old.buf);
                new.pos = old.pos % new.len;
                new.store = old.store;
            }
        }
        for (new, old) in self.allpasses.iter_mut().flatten().zip(old.allpasses.iter_mut().flatten()) {
            if new.buf.len() == old.buf.len() {
                std::mem::swap(&mut new.buf, &mut old.buf);
                new.pos = old.pos % new.len;
            }
        }
        self.predelay.adopt(&mut old.predelay);
    }
}

/// Synced delay times in beats, by option index (0 is free time in ms).
const SYNC_BEATS: [f64; 12] = [0.0, 0.125, 0.25, 0.375, 1.0 / 3.0, 0.5, 0.75, 2.0 / 3.0, 1.0, 1.5, 2.0, 4.0];
/// The longest echo: a whole note at 40 BPM.
const MAX_SECONDS: f32 = 6.5;

pub struct Delay {
    lines: [DelayLine; 2],
    sr: f32,
    time_ms: f32,
    sync_beats: Option<f64>,
    ping_pong: bool,
    feedback: f32,
    lowpass: f32,
    damped: [f32; 2],
    mix: f32,
    /// The delay in samples, gliding to a new time instead of jumping.
    current: f32,
}

impl Delay {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut delay = Self {
            lines: [DelayLine::new(MAX_SECONDS, sr), DelayLine::new(MAX_SECONDS, sr)],
            sr,
            time_ms: 0.0,
            sync_beats: None,
            ping_pong: false,
            feedback: 0.0,
            lowpass: 1.0,
            damped: [0.0; 2],
            mix: 0.0,
            current: 0.0,
        };
        delay.configure(p);
        delay
    }

    pub fn configure(&mut self, p: &dyn Params) {
        let sync = choice(p, "sync", 6.0).min(SYNC_BEATS.len() - 1);
        self.time_ms = p.or("time", 375.0).clamp(1.0, 2000.0) as f32;
        self.sync_beats = (sync > 0).then_some(SYNC_BEATS[sync]);
        self.ping_pong = choice(p, "mode", 0.0) == 1;
        self.feedback = (p.or("fb", 35.0) / 100.0).clamp(0.0, 0.95) as f32;
        self.lowpass = 1.0 - (-TAU * p.or("filter", 6000.0).clamp(100.0, 20_000.0) as f32 / self.sr).exp();
        self.mix = (p.or("mix", 20.0) / 100.0).clamp(0.0, 1.0) as f32;
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], ctx: &FxContext) {
        let max = MAX_SECONDS * self.sr - 4.0;
        let target = match self.sync_beats {
            Some(beats) => (beats * ctx.samples_per_beat) as f32,
            None => self.time_ms * 0.001 * self.sr,
        }
        .clamp(2.0, max);
        if self.current <= 0.0 {
            self.current = target;
        }
        let glide = 1.0 - (-1.0 / (0.05 * self.sr)).exp();
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.current += (target - self.current) * glide;
            let echo = [self.lines[0].read(self.current - 1.0), self.lines[1].read(self.current - 1.0)];
            for (d, e) in self.damped.iter_mut().zip(echo) {
                *d += (e - *d) * self.lowpass;
            }
            let [dl, dr] = self.damped;
            if self.ping_pong {
                // The input enters on the left; each echo crosses to the other side.
                self.lines[0].push((*l + *r) * 0.5 + dr * self.feedback);
                self.lines[1].push(dl * self.feedback);
            } else {
                self.lines[0].push(*l + dl * self.feedback);
                self.lines[1].push(*r + dr * self.feedback);
            }
            *l = *l * (1.0 - self.mix) + echo[0] * self.mix;
            *r = *r * (1.0 - self.mix) + echo[1] * self.mix;
        }
    }

    pub fn adopt(&mut self, old: &mut Delay) {
        for (new, old) in self.lines.iter_mut().zip(old.lines.iter_mut()) {
            new.adopt(old);
        }
        self.damped = old.damped;
        self.current = old.current;
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;

    fn impulse(frames: usize) -> Vec<f32> {
        let mut x = vec![0.0; frames];
        x[0] = 1.0;
        x
    }

    #[test]
    fn the_reverb_rings_and_dies_away() {
        let mut verb = AudioFx::build("reverb", SR, &params(&[("decay", 1.0), ("mix", 100.0), ("predelay", 0.0)])).unwrap();
        let (l, r) = run(&mut verb, &impulse(96_000));
        let early = rms(&l[9_600..19_200]);
        let late = rms(&l[72_000..81_600]);
        assert!(early > 1e-4 && late < early * 0.05, "{early} {late}");
        assert!(peak(&l) < 1.0 && l != r, "bounded, and wide");
        let mut dry = AudioFx::build("reverb", SR, &params(&[("mix", 0.0)])).unwrap();
        let input = sine(440.0, 0.5, 4800);
        assert_eq!(run(&mut dry, &input).0, input);
    }

    #[test]
    fn echoes_arrive_on_time_and_fade() {
        let p = [("sync", 0.0), ("time", 100.0), ("fb", 50.0), ("filter", 20_000.0), ("mix", 100.0)];
        let mut delay = AudioFx::build("delay", SR, &params(&p)).unwrap();
        let (l, _) = run(&mut delay, &impulse(24_000));
        let loudest = |from: usize, to: usize| (from..to).max_by(|a, b| l[*a].abs().total_cmp(&l[*b].abs())).unwrap();
        assert!(loudest(1, 7000).abs_diff(4800) <= 1, "first echo at 100 ms: {}", loudest(1, 7000));
        let second = loudest(7000, 12_000);
        assert!(second.abs_diff(9600) <= 2);
        assert!((l[second].abs() / l[4800].abs() - 0.5).abs() < 0.1);
    }

    #[test]
    fn synced_ping_pong_crosses_sides() {
        // A quarter note at 120 BPM (24 000 samples a beat in the test context) is half a second.
        let p = [("sync", 8.0), ("mode", 1.0), ("fb", 60.0), ("filter", 20_000.0), ("mix", 100.0)];
        let mut delay = AudioFx::build("delay", SR, &params(&p)).unwrap();
        let (l, r) = run(&mut delay, &impulse(60_000));
        assert!(l[24_000].abs() > 0.3 && r[24_000].abs() < 1e-3, "first echo on the left");
        assert!(r[48_000].abs() > 0.1 && l[48_000].abs() < 1e-3, "second on the right");
    }
}
