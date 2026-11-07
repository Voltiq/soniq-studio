//! Building blocks the instruments share: envelopes, filters, oscillators,
//! noise and panning. All plain data, so voices can be copied and swapped
//! on the audio thread.

use std::f32::consts::{FRAC_PI_4, PI, SQRT_2, TAU};

pub(crate) fn midi_hz(pitch: f32) -> f32 {
    440.0 * ((pitch - 69.0) / 12.0).exp2()
}

pub(crate) fn db_gain(db: f64) -> f32 {
    if db <= -60.0 { 0.0 } else { 10f64.powf(db / 20.0) as f32 }
}

/// Fast sine of a phase in turns (0–1 is one period), within about 0.1 %.
pub(crate) fn sin_turns(phase: f32) -> f32 {
    let p = phase - phase.floor();
    // Parabola on [-π, π) with one refinement step (sin(x + π) = −sin x).
    let x = p * TAU - PI;
    let y = (4.0 / PI) * x - (4.0 / (PI * PI)) * x * x.abs();
    -(0.225 * (y * y.abs() - y) + y)
}

/// Left and right gains for a pan of −1…1, equal power, unity in the centre.
pub(crate) fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (angle.cos() * SQRT_2, angle.sin() * SQRT_2)
}

/// One-pole low-pass coefficient for a cutoff in Hz.
pub(crate) fn one_pole(cutoff: f32, sr: f32) -> f32 {
    1.0 - (-TAU * cutoff.min(sr * 0.49) / sr).exp()
}

/// Per-sample factor that decays to −60 dB in `seconds`.
pub(crate) fn decay_coef(seconds: f32, sr: f32) -> f32 {
    (-6.9 / (seconds.max(0.0005) * sr)).exp()
}

/// Envelope settings: linear attack, exponential decay and release.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Adsr {
    attack_step: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

impl Adsr {
    /// Times in seconds, sustain 0–1.
    pub fn new(attack: f32, decay: f32, sustain: f32, release: f32, sr: f32) -> Self {
        Self {
            attack_step: 1.0 / (attack.max(0.0001) * sr),
            decay: decay_coef(decay, sr),
            sustain: sustain.clamp(0.0, 1.0),
            release: decay_coef(release, sr),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Env {
    pub stage: Stage,
    pub level: f32,
}

impl Env {
    /// Starts (or restarts) from the current level, so a retrigger never clicks.
    pub fn start(&mut self) {
        self.stage = Stage::Attack;
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn next(&mut self, s: &Adsr) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += s.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level = s.sustain + (self.level - s.sustain) * s.decay;
                if (self.level - s.sustain).abs() < 1e-4 {
                    self.stage = Stage::Sustain;
                }
            }
            // Follows sustain changes made while a note is held.
            Stage::Sustain => self.level = s.sustain,
            Stage::Release => {
                self.level *= s.release;
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// Coefficients of a state-variable filter (Simper's trapezoidal SVF), stable
/// under fast modulation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SvfCoef {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl SvfCoef {
    /// `res` 0–1 runs from a gentle Q of 0.5 to the edge of self-oscillation.
    pub fn new(cutoff: f32, res: f32, sr: f32) -> Self {
        let fc = cutoff.clamp(10.0, sr * 0.49);
        let g = (PI * fc / sr).tan();
        let k = 2.0 - 1.95 * res.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self { k, a1, a2, a3: g * a2 }
    }

    /// A filter with quality `q` (0.707 is Butterworth: two in a row make a
    /// Linkwitz-Riley crossover whose bands add back up to the input's level).
    pub fn with_q(cutoff: f32, q: f32, sr: f32) -> Self {
        let fc = cutoff.clamp(10.0, sr * 0.49);
        let g = (PI * fc / sr).tan();
        let k = 1.0 / q.max(0.05);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self { k, a1, a2, a3: g * a2 }
    }

    /// The damping: band output × `k` peaks at unity.
    pub fn k(&self) -> f32 {
        self.k
    }
}

impl Default for SvfCoef {
    fn default() -> Self {
        Self::new(1000.0, 0.0, 48_000.0)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// (low, band, high) outputs for one input sample.
    pub fn tick(&mut self, c: &SvfCoef, v0: f32) -> (f32, f32, f32) {
        let v3 = v0 - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, v0 - c.k * v1 - v2)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Wave {
    #[default]
    Saw,
    Square,
    Triangle,
    Sine,
}

impl Wave {
    /// From a parameter's option index.
    pub fn from_index(v: f64) -> Self {
        match v.round() as i64 {
            1 => Self::Square,
            2 => Self::Triangle,
            3 => Self::Sine,
            _ => Self::Saw,
        }
    }
}

/// PolyBLEP residual that rounds off a step at phase 0.
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// One sample of a band-limited waveform at `phase` (0–1), advancing `dt` per sample.
pub(crate) fn osc(wave: Wave, phase: f32, dt: f32) -> f32 {
    match wave {
        Wave::Saw => 2.0 * phase - 1.0 - blep(phase, dt),
        Wave::Square => {
            let naive = if phase < 0.5 { 1.0 } else { -1.0 };
            naive + blep(phase, dt) - blep((phase + 0.5).fract(), dt)
        }
        Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
        Wave::Sine => sin_turns(phase),
    }
}

/// xorshift32: cheap white noise in −1…1.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Noise(pub u32);

impl Noise {
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

impl Default for Noise {
    fn default() -> Self {
        Self(0x9e37_79b9)
    }
}

/// Four-point Hermite interpolation of `data` at a fractional position.
pub(crate) fn hermite(data: &[f32], pos: f64) -> f32 {
    let i = pos.floor() as isize;
    let f = (pos - i as f64) as f32;
    let at = |k: isize| -> f32 { if k < 0 { 0.0 } else { data.get(k as usize).copied().unwrap_or(0.0) } };
    let (xm1, x0, x1, x2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * f + c2) * f + c1) * f + x0
}

/// Which voice a new note takes: a free one, else the oldest released one,
/// else the oldest of all.
pub(crate) fn pick_voice(voices: impl Iterator<Item = (bool, bool, u64)>) -> usize {
    let mut best = (0usize, 3u8, u64::MAX);
    for (i, (active, held, age)) in voices.enumerate() {
        let rank = if !active { 0 } else if !held { 1 } else { 2 };
        if rank < best.1 || (rank == best.1 && age < best.2) {
            best = (i, rank, age);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_sine_is_close() {
        for i in 0..1000 {
            let p = i as f32 / 1000.0;
            assert!((sin_turns(p) - (p * TAU).sin()).abs() < 0.002, "{p}");
        }
    }

    #[test]
    fn envelope_runs_through_its_stages() {
        let s = Adsr::new(0.01, 0.1, 0.5, 0.1, 1000.0);
        let mut env = Env::default();
        env.start();
        let peak = (0..10).map(|_| env.next(&s)).fold(0.0_f32, f32::max);
        assert!((peak - 1.0).abs() < 1e-6);
        for _ in 0..500 {
            env.next(&s);
        }
        assert_eq!(env.stage, Stage::Sustain);
        env.release();
        for _ in 0..200 {
            env.next(&s);
        }
        assert!(env.is_idle());
    }
}
