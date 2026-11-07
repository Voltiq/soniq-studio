//! Audacity-style effects with parameters. Each one takes the selected audio
//! and returns the processed audio, which may be shorter or longer (speed,
//! tempo, truncate silence); the caller splices it back into the clip.

use serde::{Deserialize, Serialize};

use crate::dsp::{Biquad, butterworth, db_to_gain, filter_channels, gain_to_db};
use crate::spectral::{NoiseProfile, NoiseSettings, estimate_noise, reduce_noise};
use crate::stretch::{change_pitch, change_speed, time_stretch};
use crate::AudioBuffer;

/// Centre frequencies of the graphic equalizer.
pub const EQ_BANDS: [f64; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Effect {
    /// Needs a profile captured with "Get noise profile" first.
    NoiseReduction { reduction: f64, sensitivity: f64, smoothing: u32 },
    /// `sensitivity` 1–10; `max_width` is the longest click in ms.
    ClickRemoval { sensitivity: f64, max_width: f64 },
    DcOffset,
    HighPass { frequency: f64, slope: u32 },
    LowPass { frequency: f64, slope: u32 },
    /// Shelf gains in dB at 250 Hz and 4 kHz.
    BassTreble { bass: f64, treble: f64 },
    /// Gains in dB for `EQ_BANDS`.
    GraphicEq { bands: Vec<f64> },
    /// Threshold in dBFS, attack and release in ms; `makeup` normalises to −1 dBFS afterwards.
    Compressor { threshold: f64, ratio: f64, attack: f64, release: f64, makeup: bool },
    /// Ceiling in dBFS, release in ms.
    Limiter { ceiling: f64, release: f64 },
    /// Delay in seconds, decay as a factor per repeat.
    Echo { delay: f64, decay: f64 },
    /// Room size, damping and wet mix, each 0–1.
    Reverb { room: f64, damping: f64, mix: f64 },
    /// Percent faster (negative is slower); changes pitch and length.
    ChangeSpeed { percent: f64 },
    /// Percent faster; keeps the pitch.
    ChangeTempo { percent: f64 },
    /// Keeps the length.
    ChangePitch { semitones: f64 },
    /// Shortens pauses quieter than `threshold` dBFS and longer than
    /// `min_silence` seconds to `keep` seconds.
    TruncateSilence { threshold: f64, min_silence: f64, keep: f64 },
    /// Peak to `peak` dBFS.
    Normalize { peak: f64 },
    /// Integrated loudness to `target` LUFS.
    Loudness { target: f64 },
    /// Gain in dB.
    Amplify { gain: f64 },
    /// Learns the noise from the quietest moments of the audio itself, then
    /// lowers it by `reduction` dB: noise reduction without a profile.
    Denoise { reduction: f64 },
    /// Cuts the silence (quieter than `threshold` dBFS) before the first
    /// sound and after the last down to `keep` seconds.
    TrimSilence { threshold: f64, keep: f64 },
    /// Evens out a voice towards `target` LUFS, by at most `max_gain` dB.
    Level { target: f64, max_gain: f64 },
    /// Measures the audio and runs the repairs it needs, ending at `target` LUFS.
    FixAudio { target: f64 },
}

impl Effect {
    pub fn label(&self) -> &'static str {
        match self {
            Effect::NoiseReduction { .. } => "Noise reduction",
            Effect::ClickRemoval { .. } => "Click removal",
            Effect::DcOffset => "Remove DC offset",
            Effect::HighPass { .. } => "High-pass filter",
            Effect::LowPass { .. } => "Low-pass filter",
            Effect::BassTreble { .. } => "Bass and treble",
            Effect::GraphicEq { .. } => "Graphic EQ",
            Effect::Compressor { .. } => "Compressor",
            Effect::Limiter { .. } => "Limiter",
            Effect::Echo { .. } => "Echo",
            Effect::Reverb { .. } => "Reverb",
            Effect::ChangeSpeed { .. } => "Change speed",
            Effect::ChangeTempo { .. } => "Change tempo",
            Effect::ChangePitch { .. } => "Change pitch",
            Effect::TruncateSilence { .. } => "Truncate silence",
            Effect::Normalize { .. } => "Normalize",
            Effect::Loudness { .. } => "Loudness normalize",
            Effect::Amplify { .. } => "Amplify",
            Effect::Denoise { .. } => "Reduce noise (automatic)",
            Effect::TrimSilence { .. } => "Trim silence",
            Effect::Level { .. } => "Level",
            Effect::FixAudio { .. } => "Fix my audio",
        }
    }

    /// Whether the effect can change the length of the audio.
    pub fn changes_length(&self) -> bool {
        matches!(
            self,
            Effect::ChangeSpeed { .. } | Effect::ChangeTempo { .. } | Effect::TruncateSilence { .. } | Effect::TrimSilence { .. } | Effect::FixAudio { .. }
        )
    }
}

#[derive(Default)]
pub struct EffectContext<'a> {
    pub noise_profile: Option<&'a NoiseProfile>,
}

pub struct Processed {
    pub buffer: AudioBuffer,
    /// Something worth telling the user ("Removed 12 clicks").
    pub note: Option<String>,
}

/// Runs a chain of effects over `input`, one after the other.
pub fn apply_effects(input: &AudioBuffer, effects: &[Effect], ctx: &EffectContext) -> Result<Processed, String> {
    let mut buffer = input.clone();
    let mut notes = Vec::new();
    for effect in effects {
        let done = apply_effect(&buffer, effect, ctx)?;
        buffer = done.buffer;
        notes.extend(done.note);
    }
    Ok(Processed { buffer, note: (!notes.is_empty()).then(|| notes.join(" · ")) })
}

pub fn apply_effect(input: &AudioBuffer, effect: &Effect, ctx: &EffectContext) -> Result<Processed, String> {
    let rate = f64::from(input.sample_rate);
    let mut out = input.clone();
    let mut note = None;
    match effect {
        Effect::NoiseReduction { reduction, sensitivity, smoothing } => {
            let profile = ctx.noise_profile.ok_or("Get a noise profile first: select a stretch of noise only and press Get profile.")?;
            let settings = NoiseSettings {
                reduction_db: reduction.clamp(0.0, 48.0),
                sensitivity_db: sensitivity.clamp(0.0, 24.0),
                smoothing_bins: (*smoothing).min(12) as usize,
            };
            out = reduce_noise(input, profile, settings);
        }
        Effect::ClickRemoval { sensitivity, max_width } => {
            let width = ((max_width.clamp(0.1, 10.0) / 1000.0) * rate).round().max(1.0) as usize;
            let factor = 30.0 / sensitivity.clamp(1.0, 10.0) as f32;
            let mut most = 0;
            for ch in &mut out.channels {
                most = most.max(remove_clicks(ch, factor, width));
            }
            note = Some(match most {
                0 => "No clicks found".to_string(),
                1 => "Removed 1 click".to_string(),
                n => format!("Removed {n} clicks"),
            });
        }
        Effect::DcOffset => {
            for ch in &mut out.channels {
                let mean = ch.iter().map(|s| f64::from(*s)).sum::<f64>() / ch.len().max(1) as f64;
                ch.iter_mut().for_each(|s| *s -= mean as f32);
            }
        }
        Effect::HighPass { frequency, slope } => filter_channels(&mut out.channels, &butterworth(true, *frequency, *slope, rate)),
        Effect::LowPass { frequency, slope } => filter_channels(&mut out.channels, &butterworth(false, *frequency, *slope, rate)),
        Effect::BassTreble { bass, treble } => {
            let cascade = [Biquad::low_shelf(250.0, bass.clamp(-30.0, 30.0), rate), Biquad::high_shelf(4000.0, treble.clamp(-30.0, 30.0), rate)];
            filter_channels(&mut out.channels, &cascade);
        }
        Effect::GraphicEq { bands } => {
            let cascade: Vec<Biquad> = EQ_BANDS
                .iter()
                .zip(bands)
                .filter(|(f, g)| g.abs() > 0.01 && **f < rate * 0.45)
                .map(|(f, g)| Biquad::peaking(*f, g.clamp(-24.0, 24.0), 1.4, rate))
                .collect();
            filter_channels(&mut out.channels, &cascade);
        }
        Effect::Compressor { threshold, ratio, attack, release, makeup } => {
            compress(&mut out, *threshold, ratio.max(1.0), attack.max(0.1) / 1000.0, release.max(1.0) / 1000.0);
            if *makeup {
                scale_to_peak(&mut out, -1.0);
            }
        }
        Effect::Limiter { ceiling, release } => limit(&mut out, db_to_gain(ceiling.min(0.0)) as f32, release.max(1.0) / 1000.0),
        Effect::Echo { delay, decay } => {
            let d = (delay.clamp(0.001, 10.0) * rate).round() as usize;
            let decay = decay.clamp(0.0, 0.99) as f32;
            for ch in &mut out.channels {
                for i in d..ch.len() {
                    ch[i] += ch[i - d] * decay;
                }
            }
        }
        Effect::Reverb { room, damping, mix } => reverb(&mut out, *room, *damping, *mix),
        Effect::ChangeSpeed { percent } => out = change_speed(input, (1.0 + percent / 100.0).clamp(0.1, 10.0)),
        Effect::ChangeTempo { percent } => out = time_stretch(input, 1.0 / (1.0 + percent / 100.0).clamp(0.1, 10.0)),
        Effect::ChangePitch { semitones } => out = change_pitch(input, semitones.clamp(-24.0, 24.0)),
        Effect::TruncateSilence { threshold, min_silence, keep } => {
            let (shortened, removed) = truncate_silence(input, *threshold, *min_silence, *keep);
            out = shortened;
            note = Some(if removed > 0.0 { format!("Removed {removed:.1} s of silence") } else { "No long pauses found".into() });
        }
        Effect::Normalize { peak } => scale_to_peak(&mut out, peak.min(0.0)),
        Effect::Loudness { target } => {
            let lufs = crate::integrated_loudness(input).ok_or("Too short or too quiet to measure loudness: use at least half a second of sound.")?;
            scale(&mut out, db_to_gain(target - lufs) as f32);
        }
        Effect::Amplify { gain } => scale(&mut out, db_to_gain(*gain) as f32),
        Effect::Denoise { reduction } => {
            let profile = estimate_noise(input).ok_or("Found no quiet moment to learn the noise from. Select more of the recording, or use Noise reduction with a profile.")?;
            out = reduce_noise(input, &profile, NoiseSettings { reduction_db: reduction.clamp(0.0, 30.0), sensitivity_db: 6.0, smoothing_bins: 3 });
        }
        Effect::TrimSilence { threshold, keep } => {
            let (trimmed, removed) = crate::repair::trim_silence(input, *threshold, *keep);
            out = trimmed;
            note = Some(if removed > 0.0 { format!("Trimmed {removed:.1} s of silence") } else { "No silence at the ends".into() });
        }
        Effect::Level { target, max_gain } => {
            let (levelled, spread) = crate::repair::level(input, *target, max_gain.clamp(0.0, 24.0))?;
            out = levelled;
            note = Some(format!("Quiet and loud parts brought {spread:.0} dB closer"));
        }
        Effect::FixAudio { target } => {
            let plan = crate::repair::fix_plan(&crate::repair::diagnose(input), *target);
            let effects: Vec<Effect> = plan.into_iter().filter(|s| s.suggested).flat_map(|s| s.effects).collect();
            if effects.is_empty() {
                note = Some("Nothing needed fixing".into());
            } else {
                let done = apply_effects(input, &effects, ctx)?;
                out = done.buffer;
                note = done.note;
            }
        }
    }
    Ok(Processed { buffer: out, note })
}

fn scale(buffer: &mut AudioBuffer, gain: f32) {
    buffer.channels.iter_mut().flatten().for_each(|s| *s *= gain);
}

fn scale_to_peak(buffer: &mut AudioBuffer, peak_db: f64) {
    let peak = buffer.peak();
    if peak > 0.0 {
        scale(buffer, db_to_gain(peak_db) as f32 / peak);
    }
}

/// Feed-forward compressor with a 6 dB soft knee, detecting on the louder
/// channel so the stereo image never shifts.
fn compress(buffer: &mut AudioBuffer, threshold: f64, ratio: f64, attack: f64, release: f64) {
    let rate = f64::from(buffer.sample_rate);
    let a = (-1.0 / (attack * rate)).exp();
    let r = (-1.0 / (release * rate)).exp();
    let knee = 6.0;
    let slope = 1.0 / ratio - 1.0;
    let mut env = -120.0_f64;
    for i in 0..buffer.frames() {
        let level = buffer.channels.iter().map(|c| c[i].abs()).fold(0.0_f32, f32::max);
        let db = gain_to_db(f64::from(level));
        let coeff = if db > env { a } else { r };
        env = coeff * env + (1.0 - coeff) * db;
        let over = env - threshold;
        let reduction = if over <= -knee / 2.0 {
            0.0
        } else if over < knee / 2.0 {
            slope * (over + knee / 2.0).powi(2) / (2.0 * knee)
        } else {
            slope * over
        };
        let g = db_to_gain(reduction) as f32;
        buffer.channels.iter_mut().for_each(|c| c[i] *= g);
    }
}

/// Brick-wall limiter. Offline, it can look ahead freely: the gain is the
/// minimum needed over the next 5 ms, averaged over 5 ms so it never steps,
/// and released exponentially.
fn limit(buffer: &mut AudioBuffer, ceiling: f32, release: f64) {
    let n = buffer.frames();
    let rate = f64::from(buffer.sample_rate);
    let w = ((0.005 * rate) as usize).max(1);
    let needed: Vec<f32> = (0..n)
        .map(|i| {
            let peak = buffer.channels.iter().map(|c| c[i].abs()).fold(0.0_f32, f32::max);
            if peak > ceiling { ceiling / peak } else { 1.0 }
        })
        .collect();
    // Minimum over the window ahead (monotonic deque).
    let mut ahead = vec![1.0_f32; n];
    let mut deque = std::collections::VecDeque::new();
    for i in (0..n).rev() {
        while deque.back().is_some_and(|&j: &usize| needed[j] >= needed[i]) {
            deque.pop_back();
        }
        deque.push_back(i);
        while deque.front().is_some_and(|&j| j >= i + w) {
            deque.pop_front();
        }
        ahead[i] = needed[*deque.front().unwrap_or(&i)];
    }
    // Averaging the trailing window keeps every gain at or below what each
    // sample needs, because each term is a minimum over a window covering it.
    let coeff = (-1.0 / (release * rate)).exp() as f32;
    let mut sum = 0.0_f32;
    let mut env = 1.0_f32;
    for i in 0..n {
        sum += ahead[i];
        if i >= w {
            sum -= ahead[i - w];
        }
        let g = sum / (i + 1).min(w) as f32;
        env = if g < env { g } else { g + (env - g) * coeff };
        buffer.channels.iter_mut().for_each(|c| c[i] *= env);
    }
}

/// Finds short bursts whose sample-to-sample change jumps far above the
/// local average and redraws them with a smooth curve. Returns how many.
///
/// A click goes out and comes back: the burst sticks out from the straight
/// line between the samples around it. Sharp but legitimate edges (a
/// sawtooth resetting, a drum attack) are steps that do not come back, so
/// they are left alone.
pub(crate) fn remove_clicks(ch: &mut [f32], factor: f32, max_width: usize) -> usize {
    let n = ch.len();
    if n < 8 {
        return 0;
    }
    let diff: Vec<f32> = (0..n).map(|i| if i == 0 { 0.0 } else { (ch[i] - ch[i - 1]).abs() }).collect();
    let mut prefix = vec![0.0_f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + f64::from(diff[i]);
    }
    let half = 512;
    let local = |i: usize| {
        let (a, b) = (i.saturating_sub(half), (i + half).min(n));
        (prefix[b] - prefix[a]) as f32 / (b - a) as f32
    };
    let mut count = 0;
    let mut i = 2;
    while i < n - 2 {
        if diff[i] > factor * local(i).max(1e-5) {
            // The click lasts while the jumps stay large (gaps of 2 allowed).
            let start = i;
            let mut end = i;
            let mut j = i + 1;
            while j < n && j <= end + 3 {
                if diff[j] > factor * local(j).max(1e-5) {
                    end = j;
                }
                j += 1;
            }
            let (a, b) = (start.saturating_sub(2).max(1), (end + 2).min(n - 2));
            let (y0, y1) = (ch[a - 1], ch[b + 1]);
            let span = (b - a + 2) as f32;
            let excursion = ch[a..=b]
                .iter()
                .enumerate()
                .map(|(k, s)| (s - (y0 + (y1 - y0) * (k + 1) as f32 / span)).abs())
                .fold(0.0_f32, f32::max);
            if b - a <= max_width + 4 && excursion > 2.0 * (y1 - y0).abs() {
                // Cubic Hermite from the samples before to the samples after.
                let m0 = ch[a - 1] - ch[a.saturating_sub(2)];
                let m1 = ch[(b + 2).min(n - 1)] - ch[b + 1];
                for (k, sample) in ch[a..=b].iter_mut().enumerate() {
                    let t = (k + 1) as f32 / span;
                    let (t2, t3) = (t * t, t * t * t);
                    *sample = (2.0 * t3 - 3.0 * t2 + 1.0) * y0
                        + (t3 - 2.0 * t2 + t) * m0 * span
                        + (-2.0 * t3 + 3.0 * t2) * y1
                        + (t3 - t2) * m1 * span;
                }
                count += 1;
            }
            i = end + 3;
        } else {
            i += 1;
        }
    }
    count
}

/// Freeverb: eight damped combs and four allpasses per side.
fn reverb(buffer: &mut AudioBuffer, room: f64, damping: f64, mix: f64) {
    const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
    const SPREAD: usize = 23;
    let scale = f64::from(buffer.sample_rate) / 44_100.0;
    let feedback = (room.clamp(0.0, 1.0) * 0.28 + 0.7) as f32;
    let damp = (damping.clamp(0.0, 1.0) * 0.4) as f32;
    let mix = mix.clamp(0.0, 1.0) as f32;
    let frames = buffer.frames();
    let mono: Vec<f32> = (0..frames).map(|i| buffer.channels.iter().map(|c| c[i]).sum::<f32>() / buffer.channel_count().max(1) as f32).collect();

    for (side, ch) in buffer.channels.iter_mut().enumerate() {
        // The right side's delays are a little longer, for width.
        let extra = side * SPREAD;
        let sized = |len: usize| vec![0.0_f32; ((len + extra) as f64 * scale).round().max(1.0) as usize];
        let mut combs: Vec<(Vec<f32>, usize, f32)> = COMBS.iter().map(|l| (sized(*l), 0, 0.0)).collect();
        let mut allpasses: Vec<(Vec<f32>, usize)> = ALLPASSES.iter().map(|l| (sized(*l), 0)).collect();
        for (i, sample) in ch.iter_mut().enumerate() {
            let input = mono[i] * 0.015;
            let mut wet = 0.0;
            for (buf, pos, store) in &mut combs {
                let out = buf[*pos];
                *store = out * (1.0 - damp) + *store * damp;
                buf[*pos] = input + *store * feedback;
                *pos = (*pos + 1) % buf.len();
                wet += out;
            }
            for (buf, pos) in &mut allpasses {
                let delayed = buf[*pos];
                buf[*pos] = wet + delayed * 0.5;
                wet = delayed - wet;
                *pos = (*pos + 1) % buf.len();
            }
            *sample = *sample * (1.0 - mix) + wet * 3.0 * mix;
        }
    }
}

/// Shortens pauses. Returns the new audio and how many seconds were cut.
fn truncate_silence(input: &AudioBuffer, threshold_db: f64, min_silence: f64, keep: f64) -> (AudioBuffer, f64) {
    let rate = f64::from(input.sample_rate);
    let n = input.frames();
    let block = ((0.01 * rate) as usize).max(1);
    let limit = db_to_gain(threshold_db) as f32;
    let quiet: Vec<bool> = (0..n.div_ceil(block))
        .map(|b| {
            let (a, e) = (b * block, ((b + 1) * block).min(n));
            input.channels.iter().all(|c| c[a..e].iter().all(|s| s.abs() < limit))
        })
        .collect();
    let min_blocks = ((min_silence.max(0.01) * rate) as usize).div_ceil(block);
    let keep_frames = (keep.max(0.0) * rate) as usize;
    let fade = ((0.005 * rate) as usize).max(1);

    // Ranges of frames to cut: the middle of each long pause.
    let mut cuts = Vec::new();
    let mut b = 0;
    while b < quiet.len() {
        if !quiet[b] {
            b += 1;
            continue;
        }
        let start = b;
        while b < quiet.len() && quiet[b] {
            b += 1;
        }
        if b - start >= min_blocks {
            let (a, e) = (start * block, (b * block).min(n));
            if e - a > keep_frames + 2 * fade {
                let half = keep_frames / 2;
                cuts.push((a + half, e - (keep_frames - half)));
            }
        }
    }
    if cuts.is_empty() {
        return (input.clone(), 0.0);
    }
    let channels: Vec<Vec<f32>> = input
        .channels
        .iter()
        .map(|ch| {
            let mut out = Vec::with_capacity(n);
            let mut from = 0;
            for &(a, e) in &cuts {
                out.extend_from_slice(&ch[from..a]);
                // Crossfade the join so nothing clicks.
                let len = out.len();
                for k in 0..fade.min(len).min(n - e) {
                    let t = (k + 1) as f32 / (fade + 1) as f32;
                    let i = len - fade.min(len) + k;
                    out[i] = out[i] * (1.0 - t) + ch[e + k] * t;
                }
                from = e + fade.min(n - e);
            }
            out.extend_from_slice(&ch[from..]);
            out
        })
        .collect();
    let removed = n - channels.first().map_or(n, Vec::len);
    (AudioBuffer::new(input.sample_rate, channels), removed as f64 / rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amp: f32, rate: u32, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * amp).collect()
    }

    fn run(buffer: &AudioBuffer, effect: Effect) -> Processed {
        apply_effect(buffer, &effect, &EffectContext::default()).unwrap()
    }

    #[test]
    fn effects_deserialise_from_ui_json() {
        let e: Effect = serde_json::from_str(r#"{"type":"highPass","frequency":80,"slope":24}"#).unwrap();
        assert_eq!(e, Effect::HighPass { frequency: 80.0, slope: 24 });
        let e: Effect = serde_json::from_str(r#"{"type":"dcOffset"}"#).unwrap();
        assert_eq!(e, Effect::DcOffset);
        let e: Effect = serde_json::from_str(r#"{"type":"clickRemoval","sensitivity":5,"maxWidth":1}"#).unwrap();
        assert!(matches!(e, Effect::ClickRemoval { .. }));
    }

    #[test]
    fn limiter_holds_the_ceiling() {
        let loud = AudioBuffer::new(44_100, vec![sine(220.0, 1.6, 44_100, 1.0)]);
        let out = run(&loud, Effect::Limiter { ceiling: -1.0, release: 50.0 });
        assert!(out.buffer.peak() <= 0.8913 + 1e-4, "peak {}", out.buffer.peak());
        assert!(out.buffer.peak() > 0.85);
    }

    #[test]
    fn compressor_reduces_loud_parts_more_than_quiet_ones() {
        let rate = 44_100;
        let mut ch = sine(220.0, 0.05, rate, 0.5);
        ch.extend(sine(220.0, 0.9, rate, 0.5));
        let out = run(&AudioBuffer::new(rate, vec![ch]), Effect::Compressor { threshold: -20.0, ratio: 4.0, attack: 5.0, release: 100.0, makeup: false });
        let quiet = out.buffer.channels[0][10_000..20_000].iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        let loud = out.buffer.channels[0][40_000..44_000].iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((quiet - 0.05).abs() < 0.005);
        // −0.9 dBFS in, threshold −20, ratio 4 → about −15 dBFS out.
        assert!(gain_to_db(f64::from(loud)) < -13.0, "{}", gain_to_db(f64::from(loud)));
    }

    #[test]
    fn clicks_are_found_and_smoothed() {
        let rate = 44_100;
        let mut ch = sine(200.0, 0.3, rate, 1.0);
        for at in [5_000, 20_000, 30_000] {
            ch[at] += 0.8;
            ch[at + 1] -= 0.6;
        }
        let out = run(&AudioBuffer::new(rate, vec![ch]), Effect::ClickRemoval { sensitivity: 5.0, max_width: 1.0 });
        assert_eq!(out.note.as_deref(), Some("Removed 3 clicks"));
        assert!(out.buffer.peak() < 0.32, "peak {}", out.buffer.peak());
    }

    #[test]
    fn sharp_edges_of_a_filtered_sawtooth_are_not_clicks() {
        // Like a synth pad: three detuned saws through a gentle two-pole low-pass.
        let rate = 44_100.0_f32;
        let a = 1.0 - (-std::f32::consts::TAU * 1300.0 / rate).exp();
        let (mut phases, mut lp) = ([0.0_f32, 0.33, 0.66], [0.0_f32; 2]);
        let ch: Vec<f32> = (0..44_100)
            .map(|_| {
                let mut x = 0.0;
                for (k, d) in [0.996_f32, 1.0, 1.004].iter().enumerate() {
                    phases[k] = (phases[k] + 220.0 * d / rate).fract();
                    x += phases[k] * 2.0 - 1.0;
                }
                lp[0] += a * (x - lp[0]);
                lp[1] += a * (lp[0] - lp[1]);
                lp[1] * 0.1
            })
            .collect();
        let out = run(&AudioBuffer::new(44_100, vec![ch.clone()]), Effect::ClickRemoval { sensitivity: 5.0, max_width: 1.0 });
        assert_eq!(out.note.as_deref(), Some("No clicks found"));
        assert_eq!(out.buffer.channels[0], ch);
    }

    #[test]
    fn truncate_silence_shortens_long_pauses() {
        let rate = 10_000;
        let mut ch = sine(100.0, 0.5, rate, 1.0);
        ch.extend(vec![0.0; 3 * rate as usize]);
        ch.extend(sine(100.0, 0.5, rate, 1.0));
        let out = run(&AudioBuffer::new(rate, vec![ch]), Effect::TruncateSilence { threshold: -40.0, min_silence: 0.5, keep: 0.5 });
        let seconds = out.buffer.duration();
        assert!((seconds - 2.5).abs() < 0.05, "{seconds}");
    }

    #[test]
    fn dc_offset_is_removed() {
        let ch: Vec<f32> = sine(100.0, 0.2, 10_000, 1.0).into_iter().map(|s| s + 0.1).collect();
        let out = run(&AudioBuffer::new(10_000, vec![ch]), Effect::DcOffset);
        let mean = out.buffer.channels[0].iter().sum::<f32>() / 10_000.0;
        assert!(mean.abs() < 1e-4);
    }
}
