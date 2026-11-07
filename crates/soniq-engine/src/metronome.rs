use std::f32::consts::TAU;

/// A short, exponentially decaying sine click. Downbeats are higher and louder.
pub struct Metronome {
    sample_rate: f32,
    phase: f32,
    step: f32,
    env: f32,
    decay: f32,
}

impl Metronome {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            step: 0.0,
            env: 0.0,
            decay: (-1.0 / (0.018 * sample_rate)).exp(),
        }
    }

    pub fn trigger(&mut self, accent: bool) {
        let freq = if accent { 1760.0 } else { 1320.0 };
        self.step = freq / self.sample_rate;
        self.phase = 0.0;
        self.env = if accent { 0.5 } else { 0.32 };
    }

    pub fn tick(&mut self) -> f32 {
        if self.env < 1.0e-4 {
            return 0.0;
        }
        let out = (self.phase * TAU).sin() * self.env;
        self.phase = (self.phase + self.step).fract();
        self.env *= self.decay;
        out
    }
}
