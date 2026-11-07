//! What a sample is, for the library: a loop or a one-shot, its tempo, its
//! key (or root note) when it is tonal, and tags from its name and folders.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use serde::{Deserialize, Serialize};

use crate::AudioBuffer;

/// Analysis window and hop for the chroma, in frames.
const SIZE: usize = 8192;
const HOP: usize = 4096;
/// Only the first stretch of a long file is listened to.
const MAX_SECONDS: f64 = 30.0;
/// Below this spectral flatness a sound counts as pitched.
const TONAL_FLATNESS: f64 = 0.12;

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SampleKind {
    OneShot,
    Loop,
}

/// A key (for loops) or a root note (for one-shots, `minor` false).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyGuess {
    /// Pitch class, 0 = C.
    pub tonic: u8,
    pub minor: bool,
    /// Just one note (a one-shot's root), not a key.
    pub root: bool,
}

impl KeyGuess {
    pub fn label(&self) -> String {
        let name = NOTE_NAMES[usize::from(self.tonic % 12)];
        if self.root {
            name.to_string()
        } else {
            format!("{name} {}", if self.minor { "minor" } else { "major" })
        }
    }

    /// Semitones (−6…+5) that move this to `song`: a minor key goes to the
    /// song's relative minor when the song is major, and the other way round.
    pub fn shift_to(&self, song: KeyGuess) -> i32 {
        let mut target = i32::from(song.tonic);
        if !self.root && self.minor != song.minor {
            target += if song.minor { 3 } else { -3 };
        }
        let d = (target - i32::from(self.tonic)).rem_euclid(12);
        if d > 5 { d - 12 } else { d }
    }
}

/// What the library shows and auditions by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleInfo {
    pub seconds: f64,
    pub channels: u16,
    pub sample_rate: u32,
    pub kind: SampleKind,
    /// A loop's tempo (from its name, or heard).
    pub bpm: Option<f64>,
    pub key: Option<KeyGuess>,
    /// Peak level, dBFS.
    pub peak_db: f64,
}

/// Listens to a sample; `name` (file name, folders) helps with tempo and key.
pub fn describe(buffer: &AudioBuffer, name: &str) -> SampleInfo {
    let seconds = buffer.duration();
    let peak = buffer.channels.iter().flatten().fold(0.0_f32, |m, x| m.max(x.abs()));
    let tags = name_tags(name);
    let looped_name = tags.iter().any(|t| t == "loop");
    let heard = if (1.0..=60.0).contains(&seconds) { crate::detect_tempo(buffer) } else { None };
    let bpm = name_bpm(name).or(heard);
    let whole_beats = bpm.is_some_and(|bpm| {
        let beats = seconds * bpm / 60.0;
        beats >= 1.9 && (beats - beats.round()).abs() < 0.06
    });
    let kind = if looped_name || (seconds >= 1.0 && whole_beats && !tags.iter().any(|t| DRUM_HITS.contains(&t.as_str()))) { SampleKind::Loop } else { SampleKind::OneShot };
    let drum = tags.iter().any(|t| DRUM_HITS.contains(&t.as_str()));
    let key = name_key(name).or_else(|| if drum { None } else { detect_key(buffer, kind == SampleKind::OneShot) });
    SampleInfo {
        seconds,
        channels: buffer.channels.len() as u16,
        sample_rate: buffer.sample_rate,
        kind,
        bpm: if kind == SampleKind::Loop { bpm } else { None },
        key,
        peak_db: if peak > 0.0 { 20.0 * f64::from(peak).log10() } else { -120.0 },
    }
}

/// Tags that mean a single drum hit (never transposed to the song's key).
const DRUM_HITS: [&str; 7] = ["kick", "snare", "clap", "hat", "tom", "cymbal", "perc"];

/// Words in names and the tag each means.
const WORDS: &[(&str, &str)] = &[
    ("kick", "kick"),
    ("kik", "kick"),
    ("bd", "kick"),
    ("bassdrum", "kick"),
    ("snare", "snare"),
    ("snr", "snare"),
    ("sd", "snare"),
    ("rimshot", "snare"),
    ("clap", "clap"),
    ("clp", "clap"),
    ("hat", "hat"),
    ("hats", "hat"),
    ("hihat", "hat"),
    ("hh", "hat"),
    ("oh", "hat"),
    ("ch", "hat"),
    ("tom", "tom"),
    ("toms", "tom"),
    ("crash", "cymbal"),
    ("ride", "cymbal"),
    ("cymbal", "cymbal"),
    ("cym", "cymbal"),
    ("china", "cymbal"),
    ("splash", "cymbal"),
    ("perc", "perc"),
    ("percussion", "perc"),
    ("shaker", "perc"),
    ("conga", "perc"),
    ("bongo", "perc"),
    ("tamb", "perc"),
    ("tambourine", "perc"),
    ("cowbell", "perc"),
    ("rim", "perc"),
    ("bass", "bass"),
    ("sub", "bass"),
    ("808", "bass"),
    ("pad", "pad"),
    ("pads", "pad"),
    ("lead", "lead"),
    ("synth", "synth"),
    ("arp", "synth"),
    ("pluck", "synth"),
    ("vox", "vocal"),
    ("vocal", "vocal"),
    ("vocals", "vocal"),
    ("voice", "vocal"),
    ("chant", "vocal"),
    ("fx", "fx"),
    ("sfx", "fx"),
    ("riser", "fx"),
    ("impact", "fx"),
    ("sweep", "fx"),
    ("whoosh", "fx"),
    ("guitar", "guitar"),
    ("gtr", "guitar"),
    ("piano", "keys"),
    ("keys", "keys"),
    ("rhodes", "keys"),
    ("chord", "chords"),
    ("chords", "chords"),
    ("drum", "drums"),
    ("drums", "drums"),
    ("break", "drums"),
    ("breakbeat", "drums"),
    ("top", "drums"),
    ("string", "strings"),
    ("strings", "strings"),
    ("violin", "strings"),
    ("brass", "brass"),
    ("horn", "brass"),
    ("sax", "brass"),
    ("trumpet", "brass"),
    ("loop", "loop"),
    ("loops", "loop"),
    ("oneshot", "one-shot"),
    ("shot", "one-shot"),
];

/// The words of a name or path: split at anything not a letter or digit,
/// between letters and digits, and at lower-to-upper case changes.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        let split = match prev {
            _ if !c.is_alphanumeric() => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word).to_lowercase());
                }
                prev = None;
                continue;
            }
            Some(p) => (p.is_ascii_digit() != c.is_ascii_digit()) || (p.is_lowercase() && c.is_uppercase()),
            None => false,
        };
        if split && !word.is_empty() {
            out.push(std::mem::take(&mut word).to_lowercase());
        }
        word.push(c);
        prev = Some(c);
    }
    if !word.is_empty() {
        out.push(word.to_lowercase());
    }
    out
}

/// Tags a name suggests ("Kick_808_02.wav" → kick, bass), in order, once each.
pub fn name_tags(name: &str) -> Vec<String> {
    let all = words(name);
    // "one shot" written as two words.
    let joined: Vec<String> = all.windows(2).map(|w| format!("{}{}", w[0], w[1])).collect();
    let mut tags: Vec<String> = Vec::new();
    for w in all.iter().chain(&joined) {
        if let Some((_, tag)) = WORDS.iter().find(|(word, _)| word == w)
            && !tags.iter().any(|t| t == tag)
        {
            tags.push((*tag).to_string());
        }
    }
    tags
}

/// A tempo written in a name: "120bpm", "120 BPM", "_120_" (60–200).
pub fn name_bpm(name: &str) -> Option<f64> {
    let all = words(name);
    let number = |w: &String| w.parse::<f64>().ok().filter(|b| (60.0..=200.0).contains(b));
    for (i, w) in all.iter().enumerate() {
        if w == "bpm"
            && let Some(b) = i.checked_sub(1).and_then(|j| number(&all[j]))
        {
            return Some(b);
        }
    }
    // A lone number in the stem that looks like a tempo ("Loop_124_Am").
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
    words(stem).iter().filter(|w| w.len() == 2 || w.len() == 3).find_map(|w| number(w).filter(|b| b.fract() == 0.0 && *b >= 70.0))
}

/// A key written in a name: "Am", "F#min", "C major", "Ebm", "_D_".
pub fn name_key(name: &str) -> Option<KeyGuess> {
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
    let parts: Vec<&str> = stem.split(|c: char| !(c.is_alphanumeric() || c == '#')).filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let mut chars = part.chars();
        let letter = chars.next()?.to_ascii_uppercase();
        let base = match letter {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => continue,
        };
        // Only the first letter may be upper case ("Am", not "AM" or "Amp").
        let rest: String = chars.collect();
        let (accidental, mode) = if let Some(r) = rest.strip_prefix('#') {
            (1, r)
        } else if let Some(r) = rest.strip_prefix('b') {
            (-1, r)
        } else {
            (0, rest.as_str())
        };
        let next = parts.get(i + 1).map(|p| p.to_lowercase());
        let minor = match mode {
            "m" | "min" | "minor" => true,
            "" | "maj" | "major" => next.as_deref().is_some_and(|n| n == "minor" || n == "min"),
            _ => continue,
        };
        // A bare letter needs company to count ("A" alone is often a take name).
        if mode.is_empty() && accidental == 0 && next.as_deref().is_none_or(|n| n != "minor" && n != "major" && n != "min" && n != "maj") {
            continue;
        }
        return Some(KeyGuess { tonic: (base + accidental + 12) as u8 % 12, minor, root: false });
    }
    None
}

/// Krumhansl–Kessler key profiles.
const MAJOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

/// The key of a pitched sound (a one-shot's root note), or `None` for drums and noise.
pub fn detect_key(buffer: &AudioBuffer, one_shot: bool) -> Option<KeyGuess> {
    let (chroma, flatness) = chroma(buffer)?;
    if flatness > TONAL_FLATNESS {
        return None;
    }
    if one_shot {
        let tonic = (0..12).max_by(|&a, &b| chroma[a].total_cmp(&chroma[b]))? as u8;
        return Some(KeyGuess { tonic, minor: false, root: true });
    }
    let corr = |profile: &[f64; 12], tonic: usize| {
        let p = |i: usize| profile[(i + 12 - tonic) % 12];
        let (mc, mp) = (chroma.iter().sum::<f64>() / 12.0, profile.iter().sum::<f64>() / 12.0);
        let (mut num, mut a, mut b) = (0.0, 0.0, 0.0);
        for (i, c) in chroma.iter().enumerate() {
            let (x, y) = (c - mc, p(i) - mp);
            num += x * y;
            a += x * x;
            b += y * y;
        }
        num / (a * b).sqrt().max(1e-12)
    };
    let mut best = (f64::MIN, 0, false);
    for tonic in 0..12 {
        for (minor, profile) in [(false, &MAJOR), (true, &MINOR)] {
            let c = corr(profile, tonic);
            if c > best.0 {
                best = (c, tonic, minor);
            }
        }
    }
    (best.0 > 0.5).then_some(KeyGuess { tonic: best.1 as u8, minor: best.2, root: false })
}

/// Energy per pitch class (55 Hz–5 kHz), and the spectral flatness over the
/// same band (0 for a pure tone, near 1 for white noise).
fn chroma(buffer: &AudioBuffer) -> Option<([f64; 12], f64)> {
    let rate = f64::from(buffer.sample_rate.max(1));
    let frames = buffer.frames().min((MAX_SECONDS * rate) as usize);
    if frames < 2048 || buffer.channels.is_empty() {
        return None;
    }
    let mono: Vec<f32> = (0..frames).map(|i| buffer.channels.iter().map(|c| c[i]).sum::<f32>() / buffer.channels.len() as f32).collect();
    let size = SIZE.min(frames.next_power_of_two() / 2).max(2048);
    let fft: Arc<dyn Fft<f32>> = FftPlanner::new().plan_fft_forward(size);
    let window: Vec<f32> = (0..size).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / size as f32).cos()).collect();
    let bin_hz = rate / size as f64;
    let (lo, hi) = ((55.0 / bin_hz).ceil() as usize, ((5000.0 / bin_hz) as usize).min(size / 2 - 1));
    let classes: Vec<usize> = (0..=hi).map(|k| ((12.0 * (k.max(1) as f64 * bin_hz / 440.0).log2()).round() as i64 + 9).rem_euclid(12) as usize).collect();
    let mut chroma = [0.0_f64; 12];
    let (mut flat_sum, mut weight) = (0.0, 0.0);
    let mut buf = vec![Complex::new(0.0, 0.0); size];
    let mut start = 0;
    while start + size <= frames.max(size) {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = Complex::new(mono.get(start + i).copied().unwrap_or(0.0) * window[i], 0.0);
        }
        fft.process(&mut buf);
        let power: Vec<f64> = buf[lo..=hi].iter().map(|c| f64::from(c.norm_sqr()) + 1e-20).collect();
        let total: f64 = power.iter().sum();
        if total > 1e-8 {
            let log_mean = power.iter().map(|p| p.ln()).sum::<f64>() / power.len() as f64;
            flat_sum += log_mean.exp() / (total / power.len() as f64) * total;
            weight += total;
            for (k, p) in (lo..=hi).zip(&power) {
                chroma[classes[k]] += p.sqrt();
            }
        }
        start += HOP;
        if start + size > frames {
            break;
        }
    }
    (weight > 0.0).then(|| (chroma, flat_sum / weight))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: &[f64], seconds: f64, rate: u32) -> AudioBuffer {
        let n = (seconds * f64::from(rate)) as usize;
        let s = (0..n).map(|i| (hz.iter().map(|f| (std::f64::consts::TAU * f * i as f64 / f64::from(rate)).sin()).sum::<f64>() / hz.len() as f64 * 0.5) as f32).collect();
        AudioBuffer::new(rate, vec![s])
    }

    #[test]
    fn names_give_tags_tempo_and_key() {
        assert_eq!(name_tags("Drums/Kick_808_02.wav"), ["drums", "kick", "bass"]);
        assert_eq!(name_tags("OpenHat one shot.wav"), ["hat", "one-shot"]);
        assert_eq!(name_bpm("Loop_124_Am.wav"), Some(124.0));
        assert_eq!(name_bpm("funky 96 BPM.flac"), Some(96.0));
        assert_eq!(name_bpm("Kick_02.wav"), None);
        assert_eq!(name_key("Loop_124_Am.wav"), Some(KeyGuess { tonic: 9, minor: true, root: false }));
        assert_eq!(name_key("pad F#min.wav"), Some(KeyGuess { tonic: 6, minor: true, root: false }));
        assert_eq!(name_key("Chords Eb major.wav"), Some(KeyGuess { tonic: 3, minor: false, root: false }));
        assert_eq!(name_key("Take A.wav"), None);
        assert_eq!(name_key("Ambient.wav"), None);
    }

    #[test]
    fn a_pitched_one_shot_gets_its_root_and_noise_gets_none() {
        let a = tone(&[440.0], 0.5, 48_000);
        assert_eq!(detect_key(&a, true), Some(KeyGuess { tonic: 9, minor: false, root: true }));
        let mut seed = 1_u32;
        let noise: Vec<f32> = (0..48_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1 << 24) as f32 - 0.5
            })
            .collect();
        assert_eq!(detect_key(&AudioBuffer::new(48_000, vec![noise]), true), None);
    }

    #[test]
    fn a_chord_loop_gets_a_key() {
        // C major and A minor triads, back and forth: the key is C major or A minor.
        let rate = 48_000;
        let c = tone(&[261.63, 329.63, 392.0, 523.25], 1.0, rate);
        let am = tone(&[220.0, 261.63, 329.63, 440.0], 1.0, rate);
        let mut all = c.channels[0].clone();
        all.extend(&am.channels[0]);
        all.extend(&c.channels[0]);
        let key = detect_key(&AudioBuffer::new(rate, vec![all]), false).unwrap();
        assert!(matches!((key.tonic, key.minor), (0, false) | (9, true)), "{}", key.label());
    }

    #[test]
    fn keys_move_the_short_way_to_the_song() {
        let key = |tonic, minor| KeyGuess { tonic, minor, root: false };
        assert_eq!(key(9, true).shift_to(key(0, false)), 0, "A minor already fits C major");
        assert_eq!(key(2, false).shift_to(key(0, false)), -2);
        assert_eq!(key(7, false).shift_to(key(0, false)), 5);
        assert_eq!(key(0, false).shift_to(key(7, false)), -5);
        let root = KeyGuess { tonic: 11, minor: false, root: true };
        assert_eq!(root.shift_to(key(0, false)), 1);
    }

    #[test]
    fn a_click_loop_is_a_loop_with_its_tempo() {
        // Two bars of clicks at 120 BPM: 4 s, 8 beats.
        let rate = 48_000;
        let mut s = vec![0.0_f32; rate as usize * 4];
        for beat in 0..8 {
            for i in 0..400 {
                s[beat * 24_000 + i] = 0.8 * (-(i as f32) / 80.0).exp() * if i % 2 == 0 { 1.0 } else { -1.0 };
            }
        }
        let info = describe(&AudioBuffer::new(rate, vec![s]), "groove.wav");
        assert_eq!(info.kind, SampleKind::Loop);
        assert!(info.bpm.is_some_and(|b| (b - 120.0).abs() < 0.5), "{:?}", info.bpm);
        let hit = describe(&tone(&[60.0], 0.4, rate), "Kick 01.wav");
        assert_eq!((hit.kind, hit.key), (SampleKind::OneShot, None), "kicks are not transposed");
    }
}
