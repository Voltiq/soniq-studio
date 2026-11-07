use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{AudioBuffer, IoError};

/// Decodes the first audio track of a file into planar f32 at its own sample rate.
pub fn decode_file(path: &Path) -> Result<AudioBuffer, IoError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if ext == "opus" {
        return crate::encode::decode_opus(path);
    }
    match decode_with_symphonia(path) {
        // Ogg files may hold Opus, which Symphonia cannot decode.
        Err(IoError::Unsupported(msg)) if ext == "ogg" || ext == "oga" => crate::encode::decode_opus(path).map_err(|_| IoError::Unsupported(msg)),
        other => other,
    }
}

fn decode_with_symphonia(path: &Path) -> Result<AudioBuffer, IoError> {
    let display = path.display().to_string();
    let file = File::open(path).map_err(|source| IoError::Open { path: display.clone(), source })?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| IoError::Unsupported(format!("{display} ({e})")))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| IoError::Unsupported(display.clone()))?;
    let track_id = track.id;
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| IoError::Unsupported(display.clone()))?;

    let decode_error = |message: String| IoError::Decode { path: display.clone(), message };
    let mut channels: Vec<Vec<f32>> = Vec::new();
    // Reused across packets; grown when a packet is larger than any before it.
    let mut scratch: Option<(SampleBuffer<f32>, u64)> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(decode_error(e.to_string())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A corrupt packet is skipped, like every player does.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(decode_error(e.to_string())),
        };
        let spec = *decoded.spec();
        sample_rate = spec.rate;
        let count = spec.channels.count();
        if channels.is_empty() {
            channels = vec![Vec::new(); count];
        }
        let needed = decoded.capacity() as u64;
        if scratch.as_ref().is_none_or(|(_, frames)| *frames < needed) {
            scratch = Some((SampleBuffer::<f32>::new(needed, spec), needed));
        }
        let Some((buf, _)) = scratch.as_mut() else { unreachable!("scratch was just filled") };
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks_exact(count) {
            for (ch, sample) in channels.iter_mut().zip(frame) {
                ch.push(*sample);
            }
        }
    }

    if channels.is_empty() || sample_rate == 0 {
        return Err(decode_error("the file contains no audio".into()));
    }
    Ok(AudioBuffer::new(sample_rate, channels))
}
