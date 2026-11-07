//! Getting audio in and out (decode, resample, summarise, encode) and
//! everything that changes samples offline: edits, effects, generators and
//! spectral processing.

mod analysis;
mod decode;
mod dsp;
mod edit;
mod effects;
mod encode;
mod generate;
mod loudness;
mod onsets;
mod peaks;
mod repair;
mod resample;
mod spectral;
mod stretch;
mod tempo;
mod wav;

pub use analysis::{Analyzer, Reading, SPECTRUM_BANDS, SPECTRUM_FLOOR};
pub use decode::decode_file;
pub use edit::{EditOp, apply_edit, delete_range, extract, insert, snap_to_zero_crossing};
pub use effects::{EQ_BANDS, Effect, EffectContext, Processed, apply_effect, apply_effects};
pub use encode::{Chapter, Encoding, ExportFormat, Metadata, chapter_time, decode_opus, write_export};
pub use generate::{ClickSound, Generator, NoiseColor, Waveform, generate};
pub use loudness::integrated_loudness;
pub use onsets::onsets;
pub use peaks::{PEAK_STEP, peaks};
pub use repair::{Diagnosis, FixKind, FixStep, LONG_PAUSE, REPAIR_MAX, diagnose, fix_plan, repair};
pub use resample::resample;
pub use spectral::{FFT_SIZE, FrequencyScale, NoiseProfile, noise_profile, spectral_gain, spectrogram};
pub use tempo::detect_tempo;
pub use wav::write_wav;

/// File extensions the decoder understands, for import dialogs.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["wav", "wave", "flac", "mp3", "ogg", "oga", "opus", "m4a", "mp4", "aac", "caf", "aif", "aiff"];

#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("Could not open {path}: {source}")]
    Open { path: String, source: std::io::Error },
    #[error("{0} is not an audio format Soniq can read")]
    Unsupported(String),
    #[error("Could not decode {path}: {message}")]
    Decode { path: String, message: String },
    #[error("Could not write {path}: {message}")]
    Write { path: String, message: String },
}

/// Planar 32-bit float audio.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl AudioBuffer {
    pub fn new(sample_rate: u32, channels: Vec<Vec<f32>>) -> Self {
        Self { sample_rate, channels }
    }

    pub fn silent(sample_rate: u32, channels: usize, frames: usize) -> Self {
        Self { sample_rate, channels: vec![vec![0.0; frames]; channels] }
    }

    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn duration(&self) -> f64 {
        self.frames() as f64 / f64::from(self.sample_rate.max(1))
    }

    pub fn peak(&self) -> f32 {
        self.channels.iter().flatten().fold(0.0_f32, |m, s| m.max(s.abs()))
    }
}
