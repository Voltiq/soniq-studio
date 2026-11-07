use std::path::Path;

use crate::{AudioBuffer, IoError};

/// Writes 32-bit float WAV, which keeps every bit of the engine's resolution.
pub fn write_wav(path: &Path, buffer: &AudioBuffer) -> Result<(), IoError> {
    let error = |message: String| IoError::Write { path: path.display().to_string(), message };
    let spec = hound::WavSpec {
        channels: buffer.channel_count().max(1) as u16,
        sample_rate: buffer.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| error(e.to_string()))?;
    for i in 0..buffer.frames() {
        for ch in &buffer.channels {
            writer.write_sample(ch[i]).map_err(|e| error(e.to_string()))?;
        }
    }
    writer.finalize().map_err(|e| error(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_wav_decodes_to_the_same_audio() {
        let dir = std::env::temp_dir().join(format!("soniq-wav-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.wav");
        let left: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin() * 0.7).collect();
        let right: Vec<f32> = left.iter().map(|s| -s).collect();
        let buffer = AudioBuffer::new(48_000, vec![left, right]);

        write_wav(&path, &buffer).unwrap();
        let decoded = crate::decode_file(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.channel_count(), 2);
        assert_eq!(decoded.frames(), 4800);
        assert!(decoded.channels[1].iter().zip(&buffer.channels[1]).all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
