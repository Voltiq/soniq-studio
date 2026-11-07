//! Finding where sounds start (transients), for slicing samples.

use crate::AudioBuffer;

/// Frames per analysis hop.
const HOP: usize = 256;
/// Two onsets are at least this far apart, in seconds.
const MIN_GAP: f64 = 0.05;

/// Frames where a new sound starts: the energy jumps by more than the
/// threshold `sensitivity` sets (0 finds only big hits, 1 finds soft ones).
/// The first slice starts at 0 unless a hit comes right at the start; at
/// most `max` onsets.
pub fn onsets(buffer: &AudioBuffer, sensitivity: f32, max: usize) -> Vec<usize> {
    let frames = buffer.frames();
    let hops = frames / HOP;
    // Energy per hop in dB, both channels together.
    let energy: Vec<f32> = (0..hops)
        .map(|h| {
            let sum: f32 = buffer.channels.iter().flat_map(|ch| &ch[h * HOP..(h + 1) * HOP]).map(|s| s * s).sum();
            10.0 * (sum / (HOP * buffer.channel_count().max(1)) as f32 + 1e-10).log10()
        })
        .collect();
    // A rise of 12 dB (sensitivity 0) down to 3 dB (sensitivity 1) over the
    // last few hops, above the noise floor.
    let threshold = 12.0 - 9.0 * sensitivity.clamp(0.0, 1.0);
    let gap = ((MIN_GAP * f64::from(buffer.sample_rate)) as usize / HOP).max(1);
    let mut found: Vec<usize> = Vec::new();
    let mut last: Option<usize> = None;
    for h in 3..hops {
        let before = energy[h - 3..h].iter().copied().fold(f32::MAX, f32::min);
        let rise = energy[h] - before;
        if rise > threshold && energy[h] > -50.0 && last.is_none_or(|l| h - l >= gap) {
            // Step back to where the rise begins.
            let floor = last.map_or(0, |l| l + 1);
            let mut start = h;
            while start > floor && energy[start - 1] - before > threshold * 0.3 {
                start -= 1;
            }
            found.push(start * HOP);
            last = Some(h);
            if found.len() >= max {
                break;
            }
        }
    }
    // Sound before the first hit gets a slice of its own.
    if found.first().is_none_or(|first| *first > gap * HOP) {
        found.insert(0, 0);
        found.truncate(max.max(1));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_each_hit_of_a_drum_loop() {
        // Four decaying hits, half a second apart, at 48 kHz.
        let sr = 48_000;
        let mut data = vec![0.0f32; sr * 2];
        for hit in 0..4 {
            let at = hit * sr / 2 + 1000;
            for i in 0..8000 {
                data[at + i] = (i as f32 * 0.3).sin() * (-(i as f32) / 1500.0).exp();
            }
        }
        let found = onsets(&AudioBuffer::new(sr as u32, vec![data]), 0.5, 16);
        // The first hit comes right at the start, so it opens the first slice.
        assert_eq!(found.len(), 4, "{found:?}");
        for (k, at) in found.iter().enumerate() {
            let expected = k * sr / 2 + 1000;
            assert!(at.abs_diff(expected) < 600, "hit {k} at {at}, expected {expected}");
        }
    }
}
