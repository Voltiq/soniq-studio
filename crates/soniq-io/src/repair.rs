//! "Fix my audio": what a recording needs, found by measuring it, and the
//! steps only it needs: trimming the silence at its ends and evening out a
//! voice. (Learning the noise from the recording's own quiet moments lives
//! with the rest of the spectral code.)

use serde::{Deserialize, Serialize};

use crate::dsp::{butterworth, db_to_gain, gain_to_db};
use crate::effects::{Effect, remove_clicks};
use crate::loudness::{k_weighting, to_lufs};
use crate::AudioBuffer;

/// Pauses longer than this, in seconds, are worth shortening.
pub const LONG_PAUSE: f64 = 1.5;
/// What silence is measured in: the loudest sample of each 10 ms block,
/// the same test Truncate silence uses.
const BLOCK: f64 = 0.01;

/// What a recording sounds like, as far as fixing it goes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnosis {
    pub seconds: f64,
    pub peak_db: f64,
    pub loudness: Option<f64>,
    /// The largest channel mean: how far the waveform sits off zero.
    pub dc_offset: f64,
    /// Energy below 60 Hz against all of it, in dB.
    pub rumble_db: f64,
    /// The background between sounds: RMS of the quietest tenth, dBFS.
    pub noise_db: f64,
    /// The sound itself: RMS of the loudest tenth, dBFS.
    pub signal_db: f64,
    pub clicks: usize,
    /// Blocks whose loudest sample stays below this are silence, dBFS.
    pub silence_db: f64,
    /// Seconds of silence before the first sound and after the last.
    pub lead: f64,
    pub tail: f64,
    /// Pauses inside it longer than [`LONG_PAUSE`], and their total length.
    pub pauses: usize,
    pub pause_seconds: f64,
}

/// One kind of repair, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FixKind {
    DcOffset,
    Rumble,
    Clicks,
    Noise,
    Ends,
    Pauses,
    Loudness,
}

/// A repair, whether the recording needs it, and the effects that do it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixStep {
    pub kind: FixKind,
    pub suggested: bool,
    pub effects: Vec<Effect>,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return -140.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn db_of(power: f64) -> f64 {
    10.0 * power.max(1e-14).log10()
}

/// Levels with DC and everything below 70 Hz taken out, so rumble cannot
/// hide the silence or the background: RMS of each 50 ms (dBFS, channels
/// together) and the loudest sample of each 10 ms (dBFS, any channel).
/// Also each channel's mean, and the energy below 60 Hz against all of it.
struct Scan {
    rms: Vec<f64>,
    peaks: Vec<f64>,
    means: Vec<f64>,
    low: f64,
    all: f64,
}

/// One pass per channel, sample by sample: no copies of long audio.
fn scan(buffer: &AudioBuffer) -> Scan {
    let rate = f64::from(buffer.sample_rate);
    let n = buffer.frames();
    let wr = ((0.05 * rate) as usize).max(1);
    let wp = ((BLOCK * rate) as usize).max(1);
    let mut sums = vec![0.0_f64; n.div_ceil(wr)];
    let mut peaks = vec![0.0_f64; n.div_ceil(wp)];
    let means: Vec<f64> = buffer.channels.iter().map(|c| c.iter().map(|s| f64::from(*s)).sum::<f64>() / c.len().max(1) as f64).collect();
    let (mut low, mut all) = (0.0_f64, 0.0_f64);
    for (ch, mean) in buffer.channels.iter().zip(&means) {
        let mut high = butterworth(true, 70.0, 24, rate);
        let mut bass = butterworth(false, 60.0, 24, rate);
        for (i, s) in ch.iter().enumerate() {
            let x = f64::from(*s);
            let h = high.iter_mut().fold(x - mean, |v, f| f.process(v));
            let b = bass.iter_mut().fold(x - mean, |v, f| f.process(v));
            sums[i / wr] += h * h;
            let peak = &mut peaks[i / wp];
            *peak = peak.max(h.abs());
            low += b * b;
            all += x * x;
        }
    }
    let channels = buffer.channel_count().max(1) as f64;
    let rms = sums.iter().enumerate().map(|(k, sum)| db_of(sum / (wr.min(n - k * wr) as f64 * channels))).collect();
    let peaks = peaks.into_iter().map(|p| gain_to_db(p.max(1e-7))).collect();
    Scan { rms, peaks, means, low, all }
}

/// Measures everything "Fix my audio" looks at.
pub fn diagnose(buffer: &AudioBuffer) -> Diagnosis {
    let rate = f64::from(buffer.sample_rate);
    let seconds = buffer.duration();
    let Scan { rms, peaks, means, low, all } = scan(buffer);

    // Digital silence (padding, edits) says nothing about the background.
    let mut rms: Vec<f64> = rms.into_iter().filter(|l| *l > -120.0).collect();
    rms.sort_by(f64::total_cmp);
    let noise_db = percentile(&rms, 0.1);
    let signal_db = percentile(&rms, 0.9);

    // Silence sits a little above the background, and well under the sound.
    let mut sorted: Vec<f64> = peaks.iter().copied().filter(|l| *l > -120.0).collect();
    sorted.sort_by(f64::total_cmp);
    let silence_db = (percentile(&sorted, 0.1) + 10.0).min(percentile(&sorted, 0.9) - 20.0).clamp(-70.0, -25.0);
    let quiet: Vec<bool> = peaks.iter().map(|p| *p < silence_db).collect();
    let lead_blocks = quiet.iter().take_while(|q| **q).count();
    let tail_blocks = if lead_blocks == quiet.len() { 0 } else { quiet.iter().rev().take_while(|q| **q).count() };
    let (mut pauses, mut pause_blocks) = (0, 0);
    let mut run = 0;
    for q in &quiet[lead_blocks..quiet.len() - tail_blocks] {
        if *q {
            run += 1;
        } else {
            if run as f64 * BLOCK >= LONG_PAUSE {
                pauses += 1;
                pause_blocks += run;
            }
            run = 0;
        }
    }

    let dc_offset = means.iter().map(|m| m.abs()).fold(0.0, f64::max);
    let rumble_db = if all > 0.0 { db_of(low / all) } else { -140.0 };

    // Clicks, counted on a copy of ten seconds at a time.
    let chunk = (10.0 * rate) as usize;
    let mut clicks = 0;
    for start in (0..buffer.frames()).step_by(chunk.max(1)) {
        let end = (start + chunk).min(buffer.frames());
        clicks += buffer.channels.iter().map(|c| remove_clicks(&mut c[start..end].to_vec(), 6.0, ((0.001 * rate).round() as usize).max(1))).max().unwrap_or(0);
    }

    Diagnosis {
        seconds,
        peak_db: gain_to_db(f64::from(buffer.peak()).max(1e-7)),
        loudness: crate::integrated_loudness(buffer),
        dc_offset,
        rumble_db,
        noise_db,
        signal_db,
        clicks,
        silence_db,
        lead: (lead_blocks as f64 * BLOCK).min(seconds),
        tail: (tail_blocks as f64 * BLOCK).min(seconds),
        pauses,
        pause_seconds: pause_blocks as f64 * BLOCK,
    }
}

/// Every repair, with the ones this recording needs marked. Shortening
/// pauses changes the timing, so it is never suggested, only offered.
pub fn fix_plan(d: &Diagnosis, target: f64) -> Vec<FixStep> {
    let step = |kind, suggested, effects| FixStep { kind, suggested, effects };
    // The noise as it will sound once the loudness step has turned everything
    // up or down, and how far under the sound it sits. Quiet moments must be
    // quieter than the sound (15 dB or more) to learn the noise from. Past 32
    // dB under, one pass has done what it can (more passes only add warble).
    let snr = d.signal_db - d.noise_db;
    let after = d.noise_db + d.loudness.map_or(0.0, |l| target - l);
    let audible_noise = d.seconds >= 1.0 && after > -58.0 && (15.0..32.0).contains(&snr);
    // Down to about −66 dBFS once levelled, or 48 dB under the sound: 6 to 18 dB.
    let mut steps = vec![
        step(FixKind::DcOffset, d.dc_offset > 0.001, vec![Effect::DcOffset]),
        step(FixKind::Rumble, d.rumble_db > -15.0, vec![Effect::HighPass { frequency: 70.0, slope: 24 }]),
        step(FixKind::Clicks, d.clicks > 0, vec![Effect::ClickRemoval { sensitivity: 5.0, max_width: 1.0 }]),
        step(FixKind::Noise, audible_noise, vec![Effect::Denoise { reduction: (after + 66.0).min(48.0 - snr).clamp(6.0, 18.0).round() }]),
        step(FixKind::Ends, d.lead > 0.5 || d.tail > 0.5, vec![Effect::TrimSilence { threshold: d.silence_db, keep: 0.25 }]),
        step(FixKind::Pauses, false, vec![Effect::TruncateSilence { threshold: d.silence_db, min_silence: LONG_PAUSE, keep: 0.8 }]),
    ];
    if let Some(lufs) = d.loudness {
        // Peaks a hair over the ceiling are the limiter's own rounding.
        let needed = (lufs - target).abs() > 0.5 || d.peak_db > -0.95;
        steps.push(step(FixKind::Loudness, needed, vec![Effect::Loudness { target }, Effect::Limiter { ceiling: -1.0, release: 50.0 }]));
    }
    steps
}

/// Cuts the silence before the first sound and after the last down to
/// `keep` seconds, fading the new edges in and out over 5 ms. Returns the
/// audio and the seconds taken off.
pub fn trim_silence(input: &AudioBuffer, threshold_db: f64, keep: f64) -> (AudioBuffer, f64) {
    let rate = f64::from(input.sample_rate);
    let n = input.frames();
    let block = ((BLOCK * rate) as usize).max(1);
    // Judged as the diagnosis judges it: rumble is not sound.
    let peaks = scan(input).peaks;
    let Some(first) = peaks.iter().position(|p| *p >= threshold_db) else {
        return (input.clone(), 0.0);
    };
    let last = peaks.iter().rposition(|p| *p >= threshold_db).unwrap_or(first);
    let keep = (keep.max(0.0) * rate) as usize;
    let from = (first * block).saturating_sub(keep);
    let to = ((last + 1) * block + keep).min(n);
    if from == 0 && to == n {
        return (input.clone(), 0.0);
    }
    let fade = ((0.005 * rate) as usize).clamp(1, (to - from) / 2 + 1);
    let channels = input
        .channels
        .iter()
        .map(|ch| {
            let mut out = ch[from..to].to_vec();
            let len = out.len();
            for k in 0..fade.min(len) {
                let g = k as f32 / fade as f32;
                if from > 0 {
                    out[k] *= g;
                }
                if to < n {
                    out[len - 1 - k] *= g;
                }
            }
            out
        })
        .collect();
    (AudioBuffer::new(input.sample_rate, channels), (n - (to - from)) as f64 / rate)
}

/// Evens out a voice, the way a careful engineer rides the fader: every
/// stretch of speech is brought towards `target` LUFS, by at most
/// `max_gain` dB up or down, and the gain holds still through pauses so the
/// room between words is not pulled up. Peaks stay under −1 dBFS. Returns
/// the audio and how far apart the quietest and loudest moves were, in dB.
pub fn level(input: &AudioBuffer, target: f64, max_gain: f64) -> Result<(AudioBuffer, f64), String> {
    const TOO_SHORT: &str = "Too short or too quiet to level: use at least a few seconds of speech.";
    let integrated = crate::integrated_loudness(input).ok_or(TOO_SHORT)?;
    let rate = f64::from(input.sample_rate);
    let n = input.frames();
    let hop = ((0.1 * rate) as usize).max(1);
    let hops = n.div_ceil(hop);
    if hops < 5 {
        return Err(TOO_SHORT.into());
    }

    // K-weighted mean square of each 100 ms, channels summed as BS.1770 does.
    let (shelf, highpass) = k_weighting(rate);
    let mut power = vec![0.0_f64; hops];
    for ch in &input.channels {
        let (mut a, mut b) = ([0.0; 4], [0.0; 4]);
        for (i, s) in ch.iter().enumerate() {
            let y = highpass.step(&mut b, shelf.step(&mut a, f64::from(*s)));
            power[i / hop] += y * y;
        }
    }
    for (h, p) in power.iter_mut().enumerate() {
        *p /= hop.min(n - h * hop) as f64;
    }

    // Speech is every 100 ms well above the background (the quietest tenth)
    // and not far under the loudest; the rest is pauses.
    let mut sorted: Vec<f64> = power.iter().map(|p| to_lufs(*p)).filter(|l| *l > -100.0).collect();
    if sorted.len() < 5 {
        return Err(TOO_SHORT.into());
    }
    sorted.sort_by(f64::total_cmp);
    let gate = (percentile(&sorted, 0.1) + 12.0).max(-60.0).min(percentile(&sorted, 0.9) - 12.0);
    let speech: Vec<bool> = power.iter().map(|p| to_lufs(*p) > gate).collect();
    // The loudness of the speech within 1.5 s of each moment of speech.
    let reach = 7;
    let wanted: Vec<Option<f64>> = (0..hops)
        .map(|h| {
            if !speech[h] {
                return None;
            }
            let (a, b) = (h.saturating_sub(reach), (h + reach + 1).min(hops));
            let (sum, count) = (a..b).filter(|&k| speech[k]).fold((0.0, 0), |(s, c), k| (s + power[k], c + 1));
            Some((target - to_lufs(sum / f64::from(count))).clamp(-max_gain, max_gain))
        })
        .collect();
    // Pauses hold the gain of the speech before them (or after, at the start).
    let first = wanted.iter().flatten().next().copied().unwrap_or(target - integrated);
    let mut held = first;
    let raw: Vec<f64> = wanted
        .iter()
        .map(|w| {
            if let Some(g) = w {
                held = *g;
            }
            held
        })
        .collect();
    // Smoothed over two seconds, so the rides are slow and never pump.
    let smooth = 10;
    let gains: Vec<f64> = (0..hops)
        .map(|h| {
            let (a, b) = (h.saturating_sub(smooth), (h + smooth + 1).min(hops));
            raw[a..b].iter().sum::<f64>() / (b - a) as f64
        })
        .collect();
    let spread = gains.iter().copied().fold(f64::MIN, f64::max) - gains.iter().copied().fold(f64::MAX, f64::min);

    // Per sample: straight lines between the centres of the hops.
    let mut out = input.clone();
    let centre = |h: usize| (h * hop + hop / 2) as f64;
    for i in 0..n {
        let x = i as f64;
        let h = (((x - (hop / 2) as f64) / hop as f64).floor().max(0.0) as usize).min(hops - 1);
        let next = (h + 1).min(hops - 1);
        let t = if next == h { 0.0 } else { ((x - centre(h)) / hop as f64).clamp(0.0, 1.0) };
        let g = db_to_gain(gains[h] + (gains[next] - gains[h]) * t) as f32;
        out.channels.iter_mut().for_each(|c| c[i] *= g);
    }
    // Land on the target exactly, then keep the peaks in.
    if let Some(now) = crate::integrated_loudness(&out) {
        let g = db_to_gain((target - now).clamp(-12.0, 12.0)) as f32;
        out.channels.iter_mut().flatten().for_each(|s| *s *= g);
    }
    if out.peak() > db_to_gain(-1.0) as f32 {
        out = crate::apply_effect(&out, &Effect::Limiter { ceiling: -1.0, release: 50.0 }, &Default::default())?.buffer;
    }
    Ok((out, spread))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, db: f64, rate: u32, seconds: f64) -> Vec<f32> {
        let amp = db_to_gain(db) as f32;
        (0..(f64::from(rate) * seconds) as usize).map(|i| (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin() * amp).collect()
    }

    fn hiss(len: usize, db: f64, seed: u32) -> Vec<f32> {
        let amp = db_to_gain(db) as f32 * 1.73;
        let mut s = seed;
        (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
            })
            .collect()
    }

    /// Hiss (about 31 dB under the tone) throughout, with a tone that comes and goes: 0.8 s quiet, 2 s
    /// sound (with three clicks), a 2 s pause, 1 s sound, 1 s quiet.
    fn recording(rate: u32) -> AudioBuffer {
        let r = rate as usize;
        let total = (6.8 * f64::from(rate)) as usize;
        let mut ch = hiss(total, -48.0, 3);
        let place = |ch: &mut Vec<f32>, at: f64, seconds: f64| {
            let start = (at * f64::from(rate)) as usize;
            for (i, s) in tone(300.0, -14.0, rate, seconds).into_iter().enumerate() {
                ch[start + i] += s;
            }
        };
        place(&mut ch, 0.8, 2.0);
        place(&mut ch, 4.8, 1.0);
        for at in [r, r * 2, r * 2 + r / 2] {
            ch[at] += 0.7;
            ch[at + 1] -= 0.5;
        }
        AudioBuffer::new(rate, vec![ch])
    }

    #[test]
    fn diagnosis_finds_the_noise_clicks_silence_and_pauses() {
        let d = diagnose(&recording(44_100));
        assert!((d.noise_db + 48.0).abs() < 3.0, "noise {}", d.noise_db);
        assert!(d.signal_db > -20.0, "signal {}", d.signal_db);
        assert_eq!(d.clicks, 3);
        assert!((d.lead - 0.8).abs() < 0.05, "lead {}", d.lead);
        assert!((d.tail - 1.0).abs() < 0.05, "tail {}", d.tail);
        assert_eq!(d.pauses, 1);
        assert!((d.pause_seconds - 2.0).abs() < 0.05, "{}", d.pause_seconds);
        assert!(d.rumble_db < -20.0, "a 300 Hz tone and hiss have no rumble: {}", d.rumble_db);
        let plan = fix_plan(&d, -16.0);
        let suggested: Vec<FixKind> = plan.iter().filter(|s| s.suggested).map(|s| s.kind).collect();
        assert_eq!(suggested, [FixKind::Clicks, FixKind::Noise, FixKind::Ends, FixKind::Loudness]);
    }

    #[test]
    fn rumble_does_not_hide_the_noise_or_the_silence() {
        let rate = 44_100;
        let mut input = recording(rate);
        for (i, s) in input.channels[0].iter_mut().enumerate() {
            *s += 0.006 + 0.04 * (std::f32::consts::TAU * 25.0 * i as f32 / rate as f32).sin();
        }
        let d = diagnose(&input);
        assert!((d.lead - 0.8).abs() < 0.05 && (d.tail - 1.0).abs() < 0.05, "{d:?}");
        assert!((d.noise_db + 48.0).abs() < 3.0, "{d:?}");
        let suggested: Vec<FixKind> = fix_plan(&d, -16.0).iter().filter(|s| s.suggested).map(|s| s.kind).collect();
        assert_eq!(suggested, [FixKind::DcOffset, FixKind::Rumble, FixKind::Clicks, FixKind::Noise, FixKind::Ends, FixKind::Loudness]);
        // Trimming hears the same silence, rumble or not.
        let (_, removed) = trim_silence(&input, d.silence_db, 0.25);
        assert!((removed - 1.3).abs() < 0.05, "{removed}");
    }

    #[test]
    fn a_clean_recording_needs_almost_nothing() {
        let rate = 44_100;
        let d = diagnose(&AudioBuffer::new(rate, vec![tone(440.0, -19.0, rate, 3.0)]));
        let plan = fix_plan(&d, -16.0);
        let suggested: Vec<FixKind> = plan.iter().filter(|s| s.suggested).map(|s| s.kind).collect();
        assert_eq!(suggested, [FixKind::Loudness], "{d:?}");
    }

    #[test]
    fn fixing_the_recording_does_what_the_plan_says() {
        let input = recording(44_100);
        let out = crate::apply_effect(&input, &Effect::FixAudio { target: -16.0 }, &Default::default()).unwrap();
        // The ends are trimmed to a quarter second each: 6.8 − 0.55 − 0.75.
        assert!((out.buffer.duration() - 5.5).abs() < 0.05, "{}", out.buffer.duration());
        let lufs = crate::integrated_loudness(&out.buffer).unwrap();
        assert!((lufs + 16.0).abs() < 1.0, "{lufs}");
        assert!(out.buffer.peak() <= db_to_gain(-1.0) as f32 + 1e-4);
        let (before, after) = (diagnose(&input), diagnose(&out.buffer));
        let under = |d: &Diagnosis| d.signal_db - d.noise_db;
        assert!(under(&after) > under(&before) + 10.0, "the hiss is further under the sound: {} → {}", under(&before), under(&after));
        assert_eq!(after.clicks, 0);
        assert!(out.note.as_deref().is_some_and(|n| n.contains("3 clicks")), "{:?}", out.note);
    }

    #[test]
    fn a_second_fix_finds_nothing_more_to_do() {
        let rate = 44_100;
        let mut rumbling = recording(rate);
        for (i, s) in rumbling.channels[0].iter_mut().enumerate() {
            *s += 0.006 + 0.04 * (std::f32::consts::TAU * 25.0 * i as f32 / rate as f32).sin();
        }
        for input in [recording(rate), rumbling] {
            let once = crate::apply_effect(&input, &Effect::FixAudio { target: -16.0 }, &Default::default()).unwrap().buffer;
            let d = diagnose(&once);
            let again: Vec<FixKind> = fix_plan(&d, -16.0).iter().filter(|s| s.suggested).map(|s| s.kind).collect();
            assert!(again.is_empty(), "{again:?} {d:?}");
        }
    }

    #[test]
    fn trimming_keeps_a_little_of_the_silence() {
        let rate = 10_000;
        let mut ch = vec![0.0; rate as usize];
        ch.extend(tone(100.0, -6.0, rate, 1.0));
        ch.extend(vec![0.0; 2 * rate as usize]);
        let (out, removed) = trim_silence(&AudioBuffer::new(rate, vec![ch]), -40.0, 0.25);
        // A tone that stops dead rings for a few ms through the 70 Hz filter: a little more is kept.
        assert!((out.duration() - 1.5).abs() < 0.05, "{}", out.duration());
        assert!((removed - 2.5).abs() < 0.05);
        assert_eq!(out.channels[0][0], 0.0, "starts from silence");
    }

    #[test]
    fn leveling_brings_quiet_and_loud_speech_together() {
        // A quiet speaker and a loud one, in a room with a little hiss.
        let rate = 16_000;
        let r = rate as usize;
        let mut ch = hiss(10 * r, -60.0, 4);
        for (at, db) in [(r / 2, -30.0), (11 * r / 2, -10.0)] {
            for (i, s) in tone(400.0, db, rate, 4.0).into_iter().enumerate() {
                ch[at + i] += s;
            }
        }
        let (out, spread) = level(&AudioBuffer::new(rate, vec![ch]), -18.0, 15.0).unwrap();
        let quiet = AudioBuffer::new(rate, vec![out.channels[0][3 * r / 2..7 * r / 2].to_vec()]);
        let loud = AudioBuffer::new(rate, vec![out.channels[0][13 * r / 2..17 * r / 2].to_vec()]);
        let (q, l) = (crate::integrated_loudness(&quiet).unwrap(), crate::integrated_loudness(&loud).unwrap());
        assert!((q - l).abs() < 2.5, "quiet {q}, loud {l}");
        assert!(spread > 15.0, "{spread}");
        assert!((crate::integrated_loudness(&out).unwrap() + 18.0).abs() < 1.0);
    }

    #[test]
    fn leveling_does_not_lift_the_pauses() {
        let rate = 16_000;
        let r = rate as usize;
        let mut ch = tone(400.0, -20.0, rate, 3.0);
        let pause = hiss(2 * r, -60.0, 9);
        ch.extend(&pause);
        ch.extend(tone(400.0, -20.0, rate, 3.0));
        let (out, _) = level(&AudioBuffer::new(rate, vec![ch]), -18.0, 12.0).unwrap();
        let rms = |x: &[f32]| gain_to_db((x.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / x.len() as f64).sqrt());
        let before = rms(&pause[r / 2..3 * r / 2]);
        let after = rms(&out.channels[0][3 * r + r / 2..4 * r + r / 2]);
        // The speech around it comes up about 6 dB; a naive leveler would lift the pause the full 12.
        assert!(after - before < 7.5, "the room between words stays down: {before} → {after}");
    }
}
