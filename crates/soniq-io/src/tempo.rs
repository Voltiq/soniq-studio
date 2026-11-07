//! Finding the tempo of a recording, so a loop can be warped to the song.

use crate::AudioBuffer;

/// Analysis hop in frames (about 11 ms at 44.1 or 48 kHz).
const HOP: usize = 512;
const SLOWEST: f64 = 60.0;
const FASTEST: f64 = 200.0;
/// How strongly the onsets must repeat at the beat (a drum loop about 0.4, a pad about 0.1).
const STEADY: f64 = 0.25;

/// The tempo of `buffer` in BPM, or `None` when it has no steady pulse.
///
/// How strongly the level rises at each moment (the onset strength) is
/// compared with itself shifted by every beat length from 60 to 200 BPM; the
/// shift where it lines up best (beats and the bars they make together),
/// gently favouring tempos near 120, is the beat, refined where the same
/// peak recurs a few beats on. A loop of up to 16 bars that comes out within
/// 0.5 % of whole bars gets the tempo that makes it exact.
pub fn detect_tempo(buffer: &AudioBuffer) -> Option<f64> {
    let (bpm, strength) = pulse(buffer)?;
    if strength < STEADY {
        return None;
    }
    // A loop (up to 16 bars, or a few beats): as many whole bars or beats as fit, exactly.
    let seconds = buffer.duration();
    let beats = seconds * bpm / 60.0;
    for (unit, most) in [(4.0, 16.0), (1.0, 8.0)] {
        let whole = (beats / unit).round();
        let exact = whole * unit * 60.0 / seconds;
        if (1.0..=most).contains(&whole) && (exact - bpm).abs() / bpm < 0.005 {
            return Some((exact * 100.0).round() / 100.0);
        }
    }
    Some((bpm * 10.0).round() / 10.0)
}

/// The beat in BPM and how strongly the onsets repeat at it (0–1).
fn pulse(buffer: &AudioBuffer) -> Option<(f64, f64)> {
    let rate = f64::from(buffer.sample_rate.max(1));
    let hops = buffer.frames() / HOP;
    let hop_secs = HOP as f64 / rate;
    if hops as f64 * hop_secs < 2.0 {
        return None;
    }
    let energy: Vec<f64> = (0..hops)
        .map(|h| {
            let sum: f32 = buffer.channels.iter().flat_map(|ch| &ch[h * HOP..(h + 1) * HOP]).map(|s| s * s).sum();
            (f64::from(sum) / (HOP * buffer.channel_count().max(1)) as f64 + 1e-9).ln()
        })
        .collect();
    // Rises of the level in nepers, less the flicker of a steady sound (under about 0.4 dB a hop).
    let rises: Vec<f64> = energy.windows(2).map(|w| (w[1] - w[0] - 0.1).max(0.0)).collect();
    // Less the average around each moment (half a second either side): slow swells are not beats.
    let reach = (0.5 / hop_secs).round() as usize;
    let mut sums = vec![0.0; rises.len() + 1];
    for (i, r) in rises.iter().enumerate() {
        sums[i + 1] = sums[i] + r;
    }
    let flux: Vec<f64> = (0..rises.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(reach), (i + reach + 1).min(rises.len()));
            rises[i] - (sums[b] - sums[a]) / (b - a) as f64
        })
        .collect();
    let corr = |lag: usize| -> f64 {
        if lag >= flux.len() {
            return 0.0;
        }
        flux.iter().zip(&flux[lag..]).map(|(a, b)| a * b).sum::<f64>() / (flux.len() - lag) as f64
    };
    let lag_of = |bpm: f64| 60.0 / bpm / hop_secs;
    let (shortest, longest) = (lag_of(FASTEST).floor() as usize, lag_of(SLOWEST).ceil() as usize);
    let score = |lag: usize| {
        let bpm = 60.0 / (lag as f64 * hop_secs);
        let prior = (-0.5 * ((bpm / 120.0).log2() / 0.9).powi(2)).exp();
        (corr(lag) + 0.5 * corr(2 * lag) + 0.25 * corr(4 * lag)) * prior
    };
    let scores: Vec<(usize, f64)> = (shortest.max(1)..=longest).map(|lag| (lag, score(lag))).collect();
    let &(best, top) = scores.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    if top <= 0.0 {
        return None;
    }
    // Between hops: the peak of a parabola through the best lag and its neighbours.
    let (a, b, c) = (score(best.saturating_sub(1)), top, score(best + 1));
    let shift = if a - 2.0 * b + c < 0.0 { 0.5 * (a - c) / (a - 2.0 * b + c) } else { 0.0 };
    let rough = best as f64 + shift.clamp(-0.5, 0.5);
    // Finer: the same peak several beats on, where one hop is a smaller part of the lag.
    let lag = [8usize, 4, 2]
        .iter()
        .find_map(|&k| {
            let centre = rough * k as f64;
            if centre + k as f64 >= flux.len() as f64 / 2.0 {
                return None;
            }
            let (c, reach) = (centre.round() as usize, (k / 2).max(2));
            let m = (c.saturating_sub(reach).max(1)..=c + reach).max_by(|&a, &b| corr(a).total_cmp(&corr(b)))?;
            let (a, b, c) = (corr(m - 1), corr(m), corr(m + 1));
            let shift = if a - 2.0 * b + c < 0.0 { 0.5 * (a - c) / (a - 2.0 * b + c) } else { 0.0 };
            Some((m as f64 + shift.clamp(-0.5, 0.5)) / k as f64)
        })
        .unwrap_or(rough);
    Some((60.0 / (lag * hop_secs), corr(best) / corr(0).max(1e-12)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decaying clicks every beat at `bpm`, a louder one on each bar.
    fn clicks(bpm: f64, seconds: f64) -> AudioBuffer {
        let sr = 44_100;
        let frames = (seconds * f64::from(sr)) as usize;
        let mut data = vec![0.0f32; frames];
        let beat = 60.0 / bpm * f64::from(sr);
        let mut k = 0;
        while (k as f64 * beat) < frames as f64 {
            let at = (k as f64 * beat) as usize;
            let level = if k % 4 == 0 { 0.9 } else { 0.5 };
            for i in 0..3000.min(frames - at) {
                data[at + i] = level * (i as f32 * 0.21).sin() * (-(i as f32) / 600.0).exp();
            }
            k += 1;
        }
        AudioBuffer::new(sr, vec![data])
    }

    #[test]
    fn finds_the_beat_of_a_click_track() {
        let found = detect_tempo(&clicks(128.0, 9.3)).unwrap();
        assert!((found - 128.0).abs() < 0.2, "{found}");
    }

    #[test]
    fn a_loop_gets_the_tempo_that_makes_it_whole_bars() {
        // Two bars at 100 BPM: 4.8 s.
        let found = detect_tempo(&clicks(100.0, 4.8)).unwrap();
        assert_eq!(found, 100.0);
    }

    #[test]
    fn a_long_recording_keeps_its_own_tempo() {
        // 24 s at 112 is 11.2 bars: not a loop, so no rounding to 110.
        let found = detect_tempo(&clicks(112.0, 24.0)).unwrap();
        assert!((found - 112.0).abs() < 0.2, "{found}");
    }

    #[test]
    fn a_pad_has_no_steady_beat() {
        // A tone that swells and fades slowly, with no attacks.
        let sr = 44_100;
        let pad: Vec<f32> = (0..sr * 8)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let swell = 0.3 + 0.2 * (std::f32::consts::TAU * 0.13 * t).sin() + 0.1 * (std::f32::consts::TAU * 0.37 * t).sin();
                swell * (std::f32::consts::TAU * 220.0 * t).sin()
            })
            .collect();
        assert_eq!(detect_tempo(&AudioBuffer::new(sr as u32, vec![pad])), None);
    }

    #[test]
    fn silence_has_no_tempo() {
        assert_eq!(detect_tempo(&AudioBuffer::silent(44_100, 1, 44_100 * 4)), None);
    }
}
