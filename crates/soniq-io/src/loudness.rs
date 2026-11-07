//! Integrated loudness per ITU-R BS.1770-4 / EBU R128, in LUFS.
//!
//! K-weighting (a high shelf plus a high-pass) per channel, mean square over
//! 400 ms blocks with 75 % overlap, then an absolute gate at −70 LUFS and a
//! relative gate 10 LU below the ungated mean. Filter coefficients are derived
//! for any sample rate, as libebur128 does.

use std::f64::consts::PI;

use crate::AudioBuffer;

#[derive(Clone, Copy)]
pub(crate) struct Biquad {
    b: [f64; 3],
    a: [f64; 3],
}

impl Biquad {
    /// One sample through the filter; `state` holds the last two inputs and outputs.
    pub(crate) fn step(&self, state: &mut [f64; 4], x: f64) -> f64 {
        let [x1, x2, y1, y2] = *state;
        let y = self.b[0] * x + self.b[1] * x1 + self.b[2] * x2 - self.a[1] * y1 - self.a[2] * y2;
        *state = [x, x1, y, y1];
        y
    }

    fn run(&self, input: impl IntoIterator<Item = f64>) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        input
            .into_iter()
            .map(|x| {
                let y = self.b[0] * x + self.b[1] * x1 + self.b[2] * x2 - self.a[1] * y1 - self.a[2] * y2;
                (x2, x1, y2, y1) = (x1, x, y1, y);
                y
            })
            .collect()
    }
}

pub(crate) fn k_weighting(rate: f64) -> (Biquad, Biquad) {
    // Stage 1: high shelf, +4 dB above ~1.7 kHz (models the head).
    let (f0, gain, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (PI * f0 / rate).tan();
    let vh = 10f64.powf(gain / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    };
    // Stage 2: high-pass at ~38 Hz (RLB weighting).
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let highpass = Biquad { b: [1.0, -2.0, 1.0], a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0] };
    (shelf, highpass)
}

pub(crate) fn to_lufs(mean_square: f64) -> f64 {
    -0.691 + 10.0 * mean_square.log10()
}

/// Integrated loudness, or `None` when the audio is shorter than one 400 ms
/// block or entirely below the −70 LUFS gate.
pub fn integrated_loudness(buffer: &AudioBuffer) -> Option<f64> {
    let rate = f64::from(buffer.sample_rate);
    let block = (0.4 * rate) as usize;
    let hop = block / 4;
    let frames = buffer.frames();
    if block == 0 || frames < block {
        return None;
    }
    let (shelf, highpass) = k_weighting(rate);
    // Squared K-weighted signal, summed over channels (L/R weight 1.0).
    let mut power = vec![0.0_f64; frames];
    for ch in &buffer.channels {
        let weighted = highpass.run(shelf.run(ch.iter().map(|&v| f64::from(v))));
        for (p, v) in power.iter_mut().zip(weighted) {
            *p += v * v;
        }
    }

    // Prefix sums make every overlapping block an O(1) lookup.
    let mut prefix = Vec::with_capacity(frames + 1);
    prefix.push(0.0);
    let mut acc = 0.0;
    for p in &power {
        acc += p;
        prefix.push(acc);
    }
    let blocks: Vec<f64> = (0..=(frames - block) / hop).map(|i| (prefix[i * hop + block] - prefix[i * hop]) / block as f64).collect();

    let above_absolute: Vec<f64> = blocks.into_iter().filter(|&z| to_lufs(z) > -70.0).collect();
    if above_absolute.is_empty() {
        return None;
    }
    let relative_gate = to_lufs(above_absolute.iter().sum::<f64>() / above_absolute.len() as f64) - 10.0;
    let gated: Vec<f64> = above_absolute.into_iter().filter(|&z| to_lufs(z) > relative_gate).collect();
    Some(to_lufs(gated.iter().sum::<f64>() / gated.len() as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_sine(rate: u32, freq: f64, dbfs: f64, seconds: f64) -> AudioBuffer {
        let amp = 10f64.powf(dbfs / 20.0);
        let n = (f64::from(rate) * seconds) as usize;
        let ch: Vec<f32> = (0..n).map(|i| (amp * (2.0 * PI * freq * i as f64 / f64::from(rate)).sin()) as f32).collect();
        AudioBuffer::new(rate, vec![ch.clone(), ch])
    }

    #[test]
    fn ebu_reference_tone_reads_minus_23() {
        // EBU Tech 3341: a stereo 1 kHz sine at −23 dBFS measures −23.0 LUFS.
        for rate in [44_100, 48_000] {
            let lufs = integrated_loudness(&stereo_sine(rate, 1000.0, -23.0, 10.0)).unwrap();
            assert!((lufs + 23.0).abs() < 0.1, "{rate} Hz: {lufs}");
        }
    }

    #[test]
    fn silence_is_unmeasurable() {
        assert_eq!(integrated_loudness(&AudioBuffer::silent(48_000, 2, 48_000)), None);
    }
}
