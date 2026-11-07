use crate::AudioBuffer;

/// Samples summarised by one peak value. At 48 kHz that is ~190 peaks per
/// second: enough for arrangement and editor zoom levels down to ~10 ms.
pub const PEAK_STEP: usize = 256;

/// Absolute peak per `step` samples, per channel.
pub fn peaks(buffer: &AudioBuffer, step: usize) -> Vec<Vec<f32>> {
    let step = step.max(1);
    buffer
        .channels
        .iter()
        .map(|ch| ch.chunks(step).map(|chunk| chunk.iter().fold(0.0_f32, |m, s| m.max(s.abs()))).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarises_each_block() {
        let buffer = AudioBuffer::new(48_000, vec![vec![0.1, -0.5, 0.2, 0.0, -0.9]]);
        assert_eq!(peaks(&buffer, 2), vec![vec![0.5, 0.2, 0.9]]);
    }
}
