//! Audacity-style destructive edits. They return a new buffer and never touch
//! the input, so the project can point a clip at the result and undo by
//! pointing it back.

use std::ops::Range;

use serde::Deserialize;

use crate::AudioBuffer;

/// Unit variants arrive as plain strings ("normalize"); `Loudness` as
/// `{"loudness": -16}` with its target in LUFS.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditOp {
    Normalize,
    Amplify,
    FadeIn,
    FadeOut,
    Reverse,
    Silence,
    Loudness(f64),
}

impl EditOp {
    pub fn label(self) -> &'static str {
        match self {
            EditOp::Normalize => "Normalize",
            EditOp::Amplify => "Amplify +3 dB",
            EditOp::FadeIn => "Fade in",
            EditOp::FadeOut => "Fade out",
            EditOp::Reverse => "Reverse",
            EditOp::Silence => "Silence",
            EditOp::Loudness(_) => "Loudness normalize",
        }
    }
}

/// Normalize target: −1 dBFS leaves headroom for inter-sample peaks.
const NORMALIZE_PEAK: f32 = 0.891;
/// +3 dB.
const AMPLIFY: f32 = 1.4125;

/// Applies `op` to the frames in `range`. Returns the new buffer and whether
/// any sample now exceeds full scale (it is kept, floats have headroom).
pub fn apply_edit(input: &AudioBuffer, op: EditOp, range: Range<usize>) -> (AudioBuffer, bool) {
    let mut out = input.clone();
    let frames = out.frames();
    let range = range.start.min(frames)..range.end.min(frames);
    if range.is_empty() {
        return (out, false);
    }
    let len = range.len() as f32;
    let peak = out.channels.iter().flat_map(|ch| ch[range.clone()].iter()).fold(0.0_f32, |m, s| m.max(s.abs()));
    let mut clipped = false;
    // Gain that brings the range to the loudness target; unity if unmeasurable.
    let loudness_gain = match op {
        EditOp::Loudness(target) => crate::integrated_loudness(&extract(input, range.clone()))
            .map_or(1.0, |lufs| 10f64.powf((target - lufs) / 20.0) as f32),
        _ => 1.0,
    };

    for ch in &mut out.channels {
        let span = &mut ch[range.clone()];
        match op {
            EditOp::Normalize => {
                if peak > 0.0 {
                    let gain = NORMALIZE_PEAK / peak;
                    span.iter_mut().for_each(|s| *s *= gain);
                }
            }
            EditOp::Amplify | EditOp::Loudness(_) => {
                let gain = if op == EditOp::Amplify { AMPLIFY } else { loudness_gain };
                for s in span.iter_mut() {
                    *s *= gain;
                    clipped |= s.abs() > 1.0;
                }
            }
            EditOp::FadeIn => span.iter_mut().enumerate().for_each(|(i, s)| *s *= i as f32 / len),
            EditOp::FadeOut => span.iter_mut().enumerate().for_each(|(i, s)| *s *= 1.0 - i as f32 / len),
            EditOp::Reverse => span.reverse(),
            EditOp::Silence => span.fill(0.0),
        }
    }
    (out, clipped)
}

/// A copy of the frames in `range`.
pub fn extract(input: &AudioBuffer, range: Range<usize>) -> AudioBuffer {
    let frames = input.frames();
    let range = range.start.min(frames)..range.end.min(frames);
    AudioBuffer::new(input.sample_rate, input.channels.iter().map(|ch| ch[range.clone()].to_vec()).collect())
}

/// The buffer without the frames in `range`.
pub fn delete_range(input: &AudioBuffer, range: Range<usize>) -> AudioBuffer {
    let frames = input.frames();
    let range = range.start.min(frames)..range.end.min(frames);
    let channels = input.channels.iter().map(|ch| ch[..range.start].iter().chain(&ch[range.end..]).copied().collect()).collect();
    AudioBuffer::new(input.sample_rate, channels)
}

/// `other` inserted at frame `at`, replacing `replace` frames there. `other` is
/// resampled to the buffer's rate; mono and stereo are matched up.
pub fn insert(input: &AudioBuffer, at: usize, replace: usize, other: &AudioBuffer) -> AudioBuffer {
    let other = crate::resample(other, input.sample_rate);
    let frames = input.frames();
    let at = at.min(frames);
    let end = (at + replace).min(frames);
    let channels = input
        .channels
        .iter()
        .enumerate()
        .map(|(i, ch)| {
            let src = other.channels.get(i).or(other.channels.first());
            let middle = src.map_or(&[][..], Vec::as_slice);
            ch[..at].iter().chain(middle).chain(&ch[end..]).copied().collect()
        })
        .collect();
    AudioBuffer::new(input.sample_rate, channels)
}

/// The zero crossing on the first channel nearest to `frame`, within
/// `radius` frames; `frame` itself if there is none.
pub fn snap_to_zero_crossing(input: &AudioBuffer, frame: usize, radius: usize) -> usize {
    let Some(ch) = input.channels.first() else { return frame };
    if ch.len() < 2 {
        return frame;
    }
    let crosses = |i: usize| i + 1 < ch.len() && (ch[i] <= 0.0) != (ch[i + 1] <= 0.0);
    (0..=radius)
        .flat_map(|d| [frame.checked_sub(d), frame.checked_add(d)])
        .flatten()
        .find(|&i| crosses(i))
        .map_or(frame, |i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer() -> AudioBuffer {
        AudioBuffer::new(48_000, vec![vec![0.1, 0.2, 0.4, 0.2, 0.1], vec![0.0, -0.1, -0.2, -0.1, 0.0]])
    }

    #[test]
    fn normalize_scales_the_range_to_minus_one_db() {
        let (out, _) = apply_edit(&buffer(), EditOp::Normalize, 0..5);
        assert!((out.peak() - NORMALIZE_PEAK).abs() < 1e-6);
    }

    #[test]
    fn edits_only_touch_the_range_and_not_the_input() {
        let input = buffer();
        let (out, _) = apply_edit(&input, EditOp::Silence, 1..3);
        assert_eq!(out.channels[0], vec![0.1, 0.0, 0.0, 0.2, 0.1]);
        assert_eq!(input.channels[0][1], 0.2);
    }

    #[test]
    fn cut_and_paste_round_trip() {
        let input = buffer();
        let piece = extract(&input, 1..3);
        let without = delete_range(&input, 1..3);
        assert_eq!(without.channels[0], vec![0.1, 0.2, 0.1]);
        let restored = insert(&without, 1, 0, &piece);
        assert_eq!(restored, input);
    }

    #[test]
    fn paste_over_a_selection_replaces_it() {
        let input = buffer();
        let one = AudioBuffer::new(48_000, vec![vec![0.9]]);
        let out = insert(&input, 1, 3, &one);
        // Mono pasted into stereo lands on both channels.
        assert_eq!(out.channels[0], vec![0.1, 0.9, 0.1]);
        assert_eq!(out.channels[1], vec![0.0, 0.9, 0.0]);
    }

    #[test]
    fn snaps_to_the_nearest_zero_crossing() {
        let wave = AudioBuffer::new(48_000, vec![vec![0.5, 0.4, 0.1, -0.2, -0.5, -0.4]]);
        assert_eq!(snap_to_zero_crossing(&wave, 1, 4), 3);
        assert_eq!(snap_to_zero_crossing(&wave, 0, 1), 0);
    }

    #[test]
    fn amplify_reports_clipping() {
        let loud = AudioBuffer::new(48_000, vec![vec![0.9, -0.9]]);
        let (_, clipped) = apply_edit(&loud, EditOp::Amplify, 0..2);
        assert!(clipped);
    }
}
