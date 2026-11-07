//! Time stretching without changing pitch (WSOLA), and the speed and pitch
//! changes built on it.

use crate::AudioBuffer;

/// Changes the length by `stretch` (output length ÷ input length) and keeps
/// the pitch. 40 ms grains are overlap-added at a fixed output hop; each
/// grain's input position is nudged by up to ±12 ms to where the waveform
/// best continues the previous grain, which avoids phasing.
pub fn time_stretch(buffer: &AudioBuffer, stretch: f64) -> AudioBuffer {
    let len = buffer.frames();
    if len == 0 || !stretch.is_finite() || (stretch - 1.0).abs() < 1e-6 {
        return buffer.clone();
    }
    let rate = f64::from(buffer.sample_rate);
    let win = (((0.04 * rate) as usize) / 2 * 2).max(64);
    let hop_out = win / 2;
    let hop_in = hop_out as f64 / stretch;
    let tolerance = (0.012 * rate) as isize;
    let out_len = (len as f64 * stretch).round() as usize;
    // Periodic Hann at 50 % overlap sums to exactly one.
    let window: Vec<f32> = (0..win).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / win as f64).cos()) as f32).collect();
    let mono: Vec<f32> = (0..len).map(|i| buffer.channels.iter().map(|c| c[i]).sum::<f32>() / buffer.channel_count().max(1) as f32).collect();

    // Output sample `o` plays input `(o - hop_out) / stretch`: grain 0 starts
    // before the audio, so the first real samples are covered by two grains
    // like all the others.
    let grains = out_len / hop_out + 2;
    let mut out = vec![vec![0.0_f32; grains * hop_out + win]; buffer.channel_count()];
    let mut prev = -(hop_in.round() as isize);
    for k in 0..grains {
        let nominal = ((k as f64 - 1.0) * hop_in).round() as isize;
        let pos = if k == 0 { prev } else { best_position(&mono, prev + hop_out as isize, nominal, tolerance, hop_out) };
        let out_pos = k * hop_out;
        for (ch, dst) in buffer.channels.iter().zip(&mut out) {
            for (i, w) in window.iter().enumerate() {
                let at = pos + i as isize;
                if at >= 0 && (at as usize) < len {
                    dst[out_pos + i] += ch[at as usize] * w;
                }
            }
        }
        prev = pos;
    }
    let channels = out.into_iter().map(|ch| ch[hop_out..hop_out + out_len].to_vec()).collect();
    AudioBuffer::new(buffer.sample_rate, channels)
}

/// The start within `nominal ± tolerance` whose next `overlap` samples best
/// match those at `natural` (where the previous grain would have continued).
fn best_position(x: &[f32], natural: isize, nominal: isize, tolerance: isize, overlap: usize) -> isize {
    let at = |i: isize| if i >= 0 && (i as usize) < x.len() { x[i as usize] } else { 0.0 };
    let score = |cand: isize, step: usize| -> f32 { (0..overlap).step_by(step).map(|i| at(natural + i as isize) * at(cand + i as isize)).sum() };
    // Coarse search, then refine around the winner.
    let coarse = (-tolerance..=tolerance).step_by(4).map(|d| nominal + d).max_by(|a, b| score(*a, 4).total_cmp(&score(*b, 4))).unwrap_or(nominal);
    (coarse - 3..=coarse + 3).max_by(|a, b| score(*a, 2).total_cmp(&score(*b, 2))).unwrap_or(coarse)
}

/// Plays the audio `factor` times faster: shorter and higher, like a tape.
pub fn change_speed(buffer: &AudioBuffer, factor: f64) -> AudioBuffer {
    if (factor - 1.0).abs() < 1e-6 {
        return buffer.clone();
    }
    let rate = buffer.sample_rate;
    let as_if = AudioBuffer::new((f64::from(rate) * factor).round().max(1.0) as u32, buffer.channels.clone());
    crate::resample(&as_if, rate)
}

/// Shifts the pitch by `semitones` and keeps the length.
pub fn change_pitch(buffer: &AudioBuffer, semitones: f64) -> AudioBuffer {
    let ratio = 2f64.powf(semitones / 12.0);
    let stretched = time_stretch(buffer, ratio);
    let mut out = change_speed(&stretched, ratio);
    // Rounding can leave a sample or two either way.
    for ch in &mut out.channels {
        ch.resize(buffer.frames(), 0.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, rate: u32, seconds: f32) -> AudioBuffer {
        let n = (rate as f32 * seconds) as usize;
        AudioBuffer::new(rate, vec![(0..n).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5).collect()])
    }

    /// Zero crossings per second ÷ 2 = frequency.
    fn frequency(buffer: &AudioBuffer) -> f32 {
        let ch = &buffer.channels[0];
        let mid = &ch[ch.len() / 4..3 * ch.len() / 4];
        let crossings = mid.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        crossings as f32 / 2.0 / (mid.len() as f32 / buffer.sample_rate as f32)
    }

    #[test]
    fn stretching_keeps_the_pitch() {
        let input = tone(440.0, 44_100, 2.0);
        let out = time_stretch(&input, 1.5);
        assert_eq!(out.frames(), 132_300);
        assert!((frequency(&out) - 440.0).abs() < 4.0, "{}", frequency(&out));
        // Level stays put (no comb-filter dips).
        let peak = out.channels[0][20_000..100_000].iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.05, "peak {peak}");
    }

    #[test]
    fn pitch_shift_keeps_the_length() {
        let input = tone(440.0, 44_100, 2.0);
        let out = change_pitch(&input, 12.0);
        assert_eq!(out.frames(), input.frames());
        assert!((frequency(&out) - 880.0).abs() < 8.0, "{}", frequency(&out));
    }

    #[test]
    fn speed_change_shortens_and_raises() {
        let out = change_speed(&tone(440.0, 44_100, 2.0), 2.0);
        assert!((out.frames() as i64 - 44_100).abs() <= 2);
        assert!((frequency(&out) - 880.0).abs() < 8.0);
    }
}
