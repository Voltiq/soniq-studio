//! Small DSP building blocks shared by the effects: RBJ biquads and
//! Butterworth cascades.

use std::f64::consts::PI;

/// A second-order section (transposed direct form II).
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn from_coefficients(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
    }

    fn omega(freq: f64, rate: f64) -> (f64, f64) {
        let w = 2.0 * PI * (freq / rate).clamp(1e-5, 0.4999);
        (w.cos(), w.sin())
    }

    pub fn low_pass(freq: f64, q: f64, rate: f64) -> Self {
        let (cos, sin) = Self::omega(freq, rate);
        let alpha = sin / (2.0 * q);
        Self::from_coefficients((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    pub fn high_pass(freq: f64, q: f64, rate: f64) -> Self {
        let (cos, sin) = Self::omega(freq, rate);
        let alpha = sin / (2.0 * q);
        Self::from_coefficients((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    pub fn peaking(freq: f64, gain_db: f64, q: f64, rate: f64) -> Self {
        let (cos, sin) = Self::omega(freq, rate);
        let a = 10f64.powf(gain_db / 40.0);
        let alpha = sin / (2.0 * q);
        Self::from_coefficients(1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
    }

    /// Shelves with slope 1 (no overshoot).
    pub fn low_shelf(freq: f64, gain_db: f64, rate: f64) -> Self {
        let (cos, sin) = Self::omega(freq, rate);
        let a = 10f64.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * 2f64.sqrt();
        let sq = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            a * ((a + 1.0) - (a - 1.0) * cos + sq),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sq),
            (a + 1.0) + (a - 1.0) * cos + sq,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sq,
        )
    }

    pub fn high_shelf(freq: f64, gain_db: f64, rate: f64) -> Self {
        let (cos, sin) = Self::omega(freq, rate);
        let a = 10f64.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * 2f64.sqrt();
        let sq = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            a * ((a + 1.0) + (a - 1.0) * cos + sq),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sq),
            (a + 1.0) - (a - 1.0) * cos + sq,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sq,
        )
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Q values of the second-order sections of a Butterworth filter.
fn butterworth_qs(order: usize) -> Vec<f64> {
    let pairs = (order / 2).max(1);
    (0..pairs).map(|k| 1.0 / (2.0 * (PI * (2 * k + 1) as f64 / (4 * pairs) as f64).cos())).collect()
}

/// A Butterworth low- or high-pass with `slope` dB per octave (12, 24, 36 or 48).
pub fn butterworth(high: bool, freq: f64, slope: u32, rate: f64) -> Vec<Biquad> {
    let order = (slope / 6).clamp(2, 8) as usize;
    butterworth_qs(order)
        .into_iter()
        .map(|q| if high { Biquad::high_pass(freq, q, rate) } else { Biquad::low_pass(freq, q, rate) })
        .collect()
}

/// Runs a cascade over every channel, each with its own filter state.
pub fn filter_channels(channels: &mut [Vec<f32>], cascade: &[Biquad]) {
    for ch in channels {
        let mut stages = cascade.to_vec();
        for s in ch.iter_mut() {
            let mut v = f64::from(*s);
            for stage in &mut stages {
                v = stage.process(v);
            }
            *s = v as f32;
        }
    }
}

pub fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

pub fn gain_to_db(gain: f64) -> f64 {
    20.0 * gain.max(1e-12).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_after(cascade: &[Biquad], freq: f64, rate: f64) -> f64 {
        let n = rate as usize;
        let mut ch = vec![(0..n).map(|i| (2.0 * PI * freq * i as f64 / rate).sin() as f32).collect::<Vec<_>>()];
        filter_channels(&mut ch, cascade);
        let tail = &ch[0][n / 2..];
        gain_to_db(f64::from(tail.iter().fold(0.0_f32, |m, s| m.max(s.abs()))))
    }

    #[test]
    fn butterworth_is_minus_three_db_at_the_cutoff() {
        let rate = 48_000.0;
        for slope in [12, 24, 48] {
            let hp = butterworth(true, 1000.0, slope, rate);
            assert!((level_after(&hp, 1000.0, rate) + 3.0).abs() < 0.3, "slope {slope}");
            assert!(level_after(&hp, 10_000.0, rate).abs() < 0.2);
        }
        // An octave below a 24 dB/oct high-pass is about 24 dB down.
        let hp = butterworth(true, 1000.0, 24, rate);
        assert!((level_after(&hp, 500.0, rate) + 24.0).abs() < 1.5);
    }

    #[test]
    fn peaking_boosts_its_centre() {
        let rate = 44_100.0;
        let eq = [Biquad::peaking(1000.0, 6.0, 1.4, rate)];
        assert!((level_after(&eq, 1000.0, rate) - 6.0).abs() < 0.2);
        assert!(level_after(&eq, 10_000.0, rate).abs() < 0.5);
    }
}
