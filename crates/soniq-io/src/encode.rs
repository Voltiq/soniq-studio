//! Exporting finished audio: WAV (16/24-bit PCM or 32-bit float), FLAC, MP3
//! (LAME), Ogg Vorbis and Opus, each with tags. Integer formats get TPDF
//! dither so quiet fades do not turn into grit.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::num::{NonZeroU8, NonZeroU32};
use std::path::Path;

use flacenc::component::BitRepr;
use flacenc::error::Verify;
use flacenc::source::MemSource;
use serde::{Deserialize, Serialize};

use crate::{AudioBuffer, IoError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Wav,
    Flac,
    Mp3,
    Ogg,
    Opus,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Wav => "wav",
            ExportFormat::Flac => "flac",
            ExportFormat::Mp3 => "mp3",
            ExportFormat::Ogg => "ogg",
            ExportFormat::Opus => "opus",
        }
    }
}

/// A named stretch of the file, in milliseconds from its start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub title: String,
    pub start_ms: u32,
    pub end_ms: u32,
}

/// Tags written into the file. Empty fields are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Metadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub track: String,
    pub comment: String,
    /// Filled in by the app from the song's markers: ID3 chapter frames in
    /// MP3, CHAPTERnnn comments in FLAC, Ogg and Opus, cue points in WAV.
    #[serde(skip)]
    pub chapters: Vec<Chapter>,
}

/// `HH:MM:SS.mmm`, as chapter comments and cue sheets write times.
pub fn chapter_time(ms: u32) -> String {
    let (h, m, s, rest) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    format!("{h:02}:{m:02}:{s:02}.{rest:03}")
}

impl Metadata {
    /// (Vorbis comment key, RIFF INFO id, value) for every field that is set.
    fn fields(&self) -> Vec<(&'static str, &'static [u8; 4], &str)> {
        [
            ("TITLE", b"INAM", &self.title),
            ("ARTIST", b"IART", &self.artist),
            ("ALBUM", b"IPRD", &self.album),
            ("DATE", b"ICRD", &self.year),
            ("GENRE", b"IGNR", &self.genre),
            ("TRACKNUMBER", b"ITRK", &self.track),
            ("COMMENT", b"ICMT", &self.comment),
        ]
        .into_iter()
        .map(|(key, id, value)| (key, id, value.trim()))
        .filter(|(_, _, value)| !value.is_empty())
        .collect()
    }

    fn vorbis_comments(&self) -> Vec<String> {
        let mut comments: Vec<String> = self.fields().into_iter().map(|(key, _, value)| format!("{key}={value}")).collect();
        for (i, chapter) in self.chapters.iter().enumerate() {
            comments.push(format!("CHAPTER{:03}={}", i + 1, chapter_time(chapter.start_ms)));
            comments.push(format!("CHAPTER{:03}NAME={}", i + 1, chapter.title));
        }
        comments
    }
}

fn default_bits() -> u16 {
    24
}

fn default_bitrate() -> u32 {
    192
}

fn default_quality() -> f32 {
    6.0
}

/// How to encode. `bits` applies to WAV and FLAC, `bitrate` (kbps) to MP3
/// and Opus, `quality` (0–10) to Ogg Vorbis.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Encoding {
    pub format: ExportFormat,
    #[serde(default = "default_bits")]
    pub bits: u16,
    #[serde(default = "default_bitrate")]
    pub bitrate: u32,
    #[serde(default = "default_quality")]
    pub quality: f32,
    #[serde(default)]
    pub metadata: Metadata,
}

impl Encoding {
    pub fn new(format: ExportFormat) -> Self {
        Self { format, bits: default_bits(), bitrate: default_bitrate(), quality: default_quality(), metadata: Metadata::default() }
    }
}

/// Triangular dither: the sum of two uniform values, ±1 LSB.
struct Dither(u32);

impl Dither {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        f64::from(self.0) / f64::from(u32::MAX) - 0.5
    }

    fn next(&mut self) -> f64 {
        self.uniform() + self.uniform()
    }
}

/// Interleaved integer samples at `bits`, dithered and clamped.
fn to_integers(buffer: &AudioBuffer, bits: u16) -> Vec<i32> {
    let max = f64::from((1u32 << (bits - 1)) - 1);
    let min = -max - 1.0;
    let mut dither = Dither(0x2545_f491);
    let mut out = Vec::with_capacity(buffer.frames() * buffer.channel_count());
    for i in 0..buffer.frames() {
        for ch in &buffer.channels {
            let v = f64::from(ch[i]) * max + dither.next();
            out.push(v.round().clamp(min, max) as i32);
        }
    }
    out
}

/// Lossy codecs take at most two channels; extra ones are dropped.
fn stereo_at_most(buffer: &AudioBuffer) -> Vec<&[f32]> {
    buffer.channels.iter().take(2).map(Vec::as_slice).collect()
}

pub fn write_export(path: &Path, buffer: &AudioBuffer, encoding: &Encoding) -> Result<(), IoError> {
    let error = |message: String| IoError::Write { path: path.display().to_string(), message };
    if buffer.frames() == 0 || buffer.channel_count() == 0 {
        return Err(error("there is no audio to write".into()));
    }
    let meta = &encoding.metadata;
    match encoding.format {
        ExportFormat::Wav => {
            if encoding.bits == 32 {
                crate::write_wav(path, buffer)?;
            } else {
                let bits = if encoding.bits == 16 { 16 } else { 24 };
                let spec = hound::WavSpec {
                    channels: buffer.channel_count() as u16,
                    sample_rate: buffer.sample_rate,
                    bits_per_sample: bits,
                    sample_format: hound::SampleFormat::Int,
                };
                let mut writer = hound::WavWriter::create(path, spec).map_err(|e| error(e.to_string()))?;
                for s in to_integers(buffer, bits) {
                    writer.write_sample(s).map_err(|e| error(e.to_string()))?;
                }
                writer.finalize().map_err(|e| error(e.to_string()))?;
            }
            append_wav_info(path, meta, buffer.sample_rate).map_err(|e| error(e.to_string()))
        }
        ExportFormat::Flac => {
            let bits = if encoding.bits == 16 { 16 } else { 24 };
            let samples = to_integers(buffer, bits);
            let bytes = encode_flac(&samples, buffer.channel_count(), usize::from(bits), buffer.sample_rate as usize).map_err(error)?;
            let bytes = with_flac_comments(bytes, meta);
            std::fs::write(path, bytes).map_err(|e| error(e.to_string()))
        }
        ExportFormat::Mp3 => {
            // MPEG audio tops out at 48 kHz.
            let buffer = if buffer.sample_rate > 48_000 { crate::resample(buffer, 48_000) } else { buffer.clone() };
            let bytes = encode_mp3(&buffer, encoding.bitrate).map_err(error)?;
            std::fs::write(path, bytes).map_err(|e| error(e.to_string()))?;
            write_id3(path, meta).map_err(error)
        }
        ExportFormat::Ogg => encode_vorbis(path, buffer, encoding.quality, meta).map_err(error),
        ExportFormat::Opus => {
            let source_rate = buffer.sample_rate;
            let buffer = if source_rate == 48_000 { buffer.clone() } else { crate::resample(buffer, 48_000) };
            let file = File::create(path).map_err(|e| error(e.to_string()))?;
            encode_opus(BufWriter::new(file), &buffer, encoding.bitrate, source_rate, meta).map_err(error)
        }
    }
}

/// flacenc pads the last fixed-size block with silence, and decoders play
/// that padding. FLAC allows any block size from 16 to 65535, so pick the
/// largest one up to 4608 that divides the length, or failing that the one
/// that pads least (a handful of samples at most for real-world lengths).
fn block_size_for(frames: usize) -> usize {
    (64..=4608).rev().min_by_key(|bs| (bs - frames % bs) % bs).unwrap_or(4096)
}

fn encode_flac(samples: &[i32], channels: usize, bits: usize, rate: usize) -> Result<Vec<u8>, String> {
    let fail = |e: &dyn std::fmt::Debug| format!("{e:?}");
    let config = flacenc::config::Encoder::default().into_verified().map_err(|(_, e)| fail(&e))?;
    let frames = samples.len() / channels.max(1);
    let source = MemSource::from_samples(samples, channels, bits, rate);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, block_size_for(frames)).map_err(|e| fail(&e))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| fail(&e))?;
    Ok(sink.as_slice().to_vec())
}

const VENDOR: &str = "Soniq Studio";

/// A Vorbis comment body: vendor, then `KEY=value` entries, little-endian lengths.
fn vorbis_comment_body(comments: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    out.extend_from_slice(VENDOR.as_bytes());
    out.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for c in comments {
        out.extend_from_slice(&(c.len() as u32).to_le_bytes());
        out.extend_from_slice(c.as_bytes());
    }
    out
}

/// Inserts a VORBIS_COMMENT metadata block after the existing ones.
fn with_flac_comments(bytes: Vec<u8>, meta: &Metadata) -> Vec<u8> {
    let comments = meta.vorbis_comments();
    if comments.is_empty() || bytes.len() < 8 || &bytes[..4] != b"fLaC" {
        return bytes;
    }
    // Walk the metadata blocks to the last one.
    let mut pos = 4;
    loop {
        let Some(header) = bytes.get(pos..pos + 4) else { return bytes };
        let len = (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        let last = header[0] & 0x80 != 0;
        if last {
            let body = vorbis_comment_body(&comments);
            let mut out = Vec::with_capacity(bytes.len() + body.len() + 4);
            out.extend_from_slice(&bytes[..pos]);
            out.push(header[0] & 0x7f);
            out.extend_from_slice(&bytes[pos + 1..pos + 4 + len]);
            // Type 4 = VORBIS_COMMENT, now the last block.
            out.push(0x80 | 4);
            out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
            out.extend_from_slice(&body);
            out.extend_from_slice(&bytes[pos + 4 + len..]);
            return out;
        }
        pos += 4 + len;
    }
}

/// Appends a LIST/INFO chunk and fixes the RIFF size.
/// Appends a RIFF chunk, padded to an even length.
fn riff_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

fn append_wav_info(path: &Path, meta: &Metadata, sample_rate: u32) -> std::io::Result<()> {
    let fields = meta.fields();
    if fields.is_empty() && meta.chapters.is_empty() {
        return Ok(());
    }
    let mut bytes = std::fs::read(path)?;
    if !meta.chapters.is_empty() {
        // Cue points with labels: how DAWs and editors read markers in a WAV.
        let mut cue = (meta.chapters.len() as u32).to_le_bytes().to_vec();
        let mut labels = b"adtl".to_vec();
        for (i, chapter) in meta.chapters.iter().enumerate() {
            let id = i as u32 + 1;
            let frame = (u64::from(chapter.start_ms) * u64::from(sample_rate) / 1000) as u32;
            for v in [id, frame] {
                cue.extend_from_slice(&v.to_le_bytes());
            }
            cue.extend_from_slice(b"data");
            for v in [0u32, 0, frame] {
                cue.extend_from_slice(&v.to_le_bytes());
            }
            let mut label = id.to_le_bytes().to_vec();
            label.extend_from_slice(chapter.title.as_bytes());
            label.push(0);
            riff_chunk(&mut labels, b"labl", &label);
        }
        riff_chunk(&mut bytes, b"cue ", &cue);
        riff_chunk(&mut bytes, b"LIST", &labels);
    }
    if fields.is_empty() {
        let riff = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff.to_le_bytes());
        return std::fs::write(path, bytes);
    }
    let mut info = b"INFO".to_vec();
    for (_, id, value) in fields {
        let mut data = value.as_bytes().to_vec();
        data.push(0);
        info.extend_from_slice(id);
        info.extend_from_slice(&(data.len() as u32).to_le_bytes());
        info.extend_from_slice(&data);
        if data.len() % 2 == 1 {
            info.push(0);
        }
    }
    bytes.extend_from_slice(b"LIST");
    bytes.extend_from_slice(&(info.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&info);
    let riff = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&riff.to_le_bytes());
    std::fs::write(path, bytes)
}

fn encode_mp3(buffer: &AudioBuffer, kbps: u32) -> Result<Vec<u8>, String> {
    use mp3lame_encoder::{Bitrate, Builder, DualPcm, FlushNoGap, MonoPcm, Quality};
    let bitrate = match kbps {
        0..=80 => Bitrate::Kbps64,
        81..=112 => Bitrate::Kbps96,
        113..=144 => Bitrate::Kbps128,
        145..=176 => Bitrate::Kbps160,
        177..=208 => Bitrate::Kbps192,
        209..=240 => Bitrate::Kbps224,
        241..=288 => Bitrate::Kbps256,
        _ => Bitrate::Kbps320,
    };
    let channels = stereo_at_most(buffer);
    let fail = |what: &str| format!("the MP3 encoder refused the {what}");
    let mut builder = Builder::new().ok_or("the MP3 encoder could not start")?;
    builder.set_num_channels(channels.len() as u8).map_err(|_| fail("channel count"))?;
    builder.set_sample_rate(buffer.sample_rate).map_err(|_| fail("sample rate"))?;
    builder.set_brate(bitrate).map_err(|_| fail("bitrate"))?;
    builder.set_quality(Quality::NearBest).map_err(|_| fail("quality"))?;
    let mut encoder = builder.build().map_err(|e| format!("the MP3 encoder could not start: {e:?}"))?;

    let clamp = |x: &[f32]| x.iter().map(|s| s.clamp(-1.0, 1.0)).collect::<Vec<f32>>();
    let mut out = Vec::new();
    const CHUNK: usize = 1 << 16;
    for start in (0..buffer.frames()).step_by(CHUNK) {
        let end = (start + CHUNK).min(buffer.frames());
        out.reserve(mp3lame_encoder::max_required_buffer_size(end - start));
        let left = clamp(&channels[0][start..end]);
        let written = if let Some(right) = channels.get(1) {
            let right = clamp(&right[start..end]);
            encoder.encode_to_vec(DualPcm { left: &left, right: &right }, &mut out)
        } else {
            encoder.encode_to_vec(MonoPcm(&left), &mut out)
        };
        written.map_err(|e| format!("MP3 encoding failed: {e:?}"))?;
    }
    out.reserve(7200);
    encoder.flush_to_vec::<FlushNoGap>(&mut out).map_err(|e| format!("MP3 encoding failed: {e:?}"))?;
    Ok(out)
}

fn write_id3(path: &Path, meta: &Metadata) -> Result<(), String> {
    use id3::TagLike;
    if meta.fields().is_empty() && meta.chapters.is_empty() {
        return Ok(());
    }
    let mut tag = id3::Tag::new();
    let set = |value: &str| (!value.trim().is_empty()).then(|| value.trim().to_string());
    if let Some(v) = set(&meta.title) {
        tag.set_title(v);
    }
    if let Some(v) = set(&meta.artist) {
        tag.set_artist(v);
    }
    if let Some(v) = set(&meta.album) {
        tag.set_album(v);
    }
    if let Some(v) = set(&meta.genre) {
        tag.set_genre(v);
    }
    if let Some(year) = meta.year.trim().get(..4).and_then(|y| y.parse::<i32>().ok()) {
        tag.set_year(year);
    }
    if let Some(track) = meta.track.split('/').next().and_then(|t| t.trim().parse::<u32>().ok()) {
        tag.set_track(track);
    }
    if let Some(v) = set(&meta.comment) {
        tag.add_frame(id3::frame::Comment { lang: "eng".into(), description: String::new(), text: v });
    }
    // Chapters (ID3v2 CHAP), listed in order by a top-level table of contents (CTOC).
    for (i, chapter) in meta.chapters.iter().enumerate() {
        tag.add_frame(id3::frame::Chapter {
            element_id: format!("chp{i}"),
            start_time: chapter.start_ms,
            end_time: chapter.end_ms,
            start_offset: u32::MAX,
            end_offset: u32::MAX,
            frames: vec![id3::Frame::text("TIT2", chapter.title.clone())],
        });
    }
    if !meta.chapters.is_empty() {
        tag.add_frame(id3::frame::TableOfContents {
            element_id: "toc".into(),
            top_level: true,
            ordered: true,
            elements: (0..meta.chapters.len()).map(|i| format!("chp{i}")).collect(),
            frames: Vec::new(),
        });
    }
    tag.write_to_path(path, id3::Version::Id3v24).map_err(|e| format!("could not write the tags: {e}"))
}

fn encode_vorbis(path: &Path, buffer: &AudioBuffer, quality: f32, meta: &Metadata) -> Result<(), String> {
    use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};
    let channels = stereo_at_most(buffer);
    let file = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    let rate = NonZeroU32::new(buffer.sample_rate).ok_or("invalid sample rate")?;
    let count = NonZeroU8::new(channels.len() as u8).ok_or("no channels")?;
    let fail = |e: vorbis_rs::VorbisError| format!("Ogg Vorbis encoding failed: {e}");
    let mut builder = VorbisEncoderBuilder::new(rate, count, file).map_err(fail)?;
    builder.bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr { target_quality: (quality / 10.0).clamp(-0.1, 1.0) });
    for comment in meta.vorbis_comments() {
        if let Some((key, value)) = comment.split_once('=') {
            builder.comment_tag(key.to_string(), value.to_string()).map_err(fail)?;
        }
    }
    let mut encoder = builder.build().map_err(fail)?;
    const CHUNK: usize = 4096;
    for start in (0..buffer.frames()).step_by(CHUNK) {
        let end = (start + CHUNK).min(buffer.frames());
        let block: Vec<&[f32]> = channels.iter().map(|c| &c[start..end]).collect();
        encoder.encode_audio_block(&block).map_err(fail)?;
    }
    let mut file = encoder.finish().map_err(fail)?;
    file.flush().map_err(|e| e.to_string())
}

/// 20 ms at 48 kHz.
const OPUS_FRAME: usize = 960;

/// Opus in Ogg (RFC 7845). `buffer` must be at 48 kHz; `source_rate` is
/// recorded in the header for players that want to resample back.
fn encode_opus<W: Write>(sink: W, buffer: &AudioBuffer, kbps: u32, source_rate: u32, meta: &Metadata) -> Result<(), String> {
    use ogg::writing::PacketWriteEndInfo;
    use unsafe_libopus::{
        OPUS_APPLICATION_AUDIO, OPUS_GET_LOOKAHEAD_REQUEST, OPUS_OK, OPUS_SET_BITRATE_REQUEST, opus_encode_float, opus_encoder_create,
        opus_encoder_ctl, opus_encoder_destroy,
    };
    let channels = stereo_at_most(buffer);
    let count = channels.len();
    let frames = buffer.frames();
    let mut err = 0;
    // SAFETY: plain C-style API; the encoder is destroyed below on every path.
    let encoder = unsafe { opus_encoder_create(48_000, count as i32, OPUS_APPLICATION_AUDIO, &mut err) };
    if encoder.is_null() || err != OPUS_OK {
        return Err(format!("the Opus encoder could not start (error {err})"));
    }
    struct Guard(*mut unsafe_libopus::OpusEncoder);
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: created by opus_encoder_create and destroyed once.
            unsafe { opus_encoder_destroy(self.0) }
        }
    }
    let _guard = Guard(encoder);
    let mut lookahead: i32 = 312;
    // SAFETY: the encoder is valid; the ctl requests take the documented argument types.
    unsafe {
        opus_encoder_ctl!(encoder, OPUS_SET_BITRATE_REQUEST, (kbps.clamp(6, 510) * 1000) as i32);
        opus_encoder_ctl!(encoder, OPUS_GET_LOOKAHEAD_REQUEST, &mut lookahead);
    }
    let pre_skip = lookahead.max(0) as usize;

    let mut writer = ogg::PacketWriter::new(sink);
    let serial = 0x536f_6e71; // "Sonq"
    let mut head = b"OpusHead".to_vec();
    head.push(1);
    head.push(count as u8);
    head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
    head.extend_from_slice(&source_rate.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes());
    head.push(0);
    let io = |e: std::io::Error| e.to_string();
    writer.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0).map_err(io)?;
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&vorbis_comment_body(&meta.vorbis_comments()));
    writer.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0).map_err(io)?;

    // The encoder delays its output by `pre_skip`; feed that much extra
    // silence so the end of the audio is encoded too.
    let total = pre_skip + frames;
    let packets = total.div_ceil(OPUS_FRAME);
    let mut pcm = vec![0.0_f32; OPUS_FRAME * count];
    let mut packet = vec![0u8; 4000];
    for p in 0..packets {
        for i in 0..OPUS_FRAME {
            let at = p * OPUS_FRAME + i;
            for (c, ch) in channels.iter().enumerate() {
                pcm[i * count + c] = ch.get(at).copied().unwrap_or(0.0);
            }
        }
        // SAFETY: `pcm` holds one frame for every channel and `packet` is large enough.
        let len = unsafe { opus_encode_float(encoder, pcm.as_ptr(), OPUS_FRAME as i32, packet.as_mut_ptr(), packet.len() as i32) };
        if len < 0 {
            return Err(format!("Opus encoding failed (error {len})"));
        }
        let last = p + 1 == packets;
        let granule = (((p + 1) * OPUS_FRAME).min(total)) as u64;
        let end = if last {
            PacketWriteEndInfo::EndStream
        } else if (p + 1) % 50 == 0 {
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        writer.write_packet(packet[..len as usize].to_vec(), serial, end, granule).map_err(io)?;
    }
    writer.into_inner().flush().map_err(io)
}

/// Decodes an Ogg Opus file to 48 kHz (Symphonia has no Opus decoder).
pub fn decode_opus(path: &Path) -> Result<AudioBuffer, IoError> {
    use unsafe_libopus::{opus_decode_float, opus_decoder_create, opus_decoder_destroy};
    let display = path.display().to_string();
    let fail = |message: String| IoError::Decode { path: display.clone(), message };
    let file = File::open(path).map_err(|source| IoError::Open { path: display.clone(), source })?;
    let mut reader = ogg::PacketReader::new(std::io::BufReader::new(file));

    let head = reader.read_packet().map_err(|e| fail(e.to_string()))?.ok_or_else(|| fail("the file is empty".into()))?;
    if head.data.len() < 19 || &head.data[..8] != b"OpusHead" {
        return Err(IoError::Unsupported(format!("{display} (not an Opus stream)")));
    }
    let count = usize::from(head.data[9]).clamp(1, 2);
    let pre_skip = usize::from(u16::from_le_bytes([head.data[10], head.data[11]]));
    let mut err = 0;
    // SAFETY: plain C-style API; the guard destroys it.
    let decoder = unsafe { opus_decoder_create(48_000, count as i32, &mut err) };
    if decoder.is_null() {
        return Err(fail(format!("the Opus decoder could not start (error {err})")));
    }
    struct Guard(*mut unsafe_libopus::OpusDecoder);
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: created by opus_decoder_create and destroyed once.
            unsafe { opus_decoder_destroy(self.0) }
        }
    }
    let _guard = Guard(decoder);
    let mut interleaved = Vec::new();
    let mut pcm = vec![0.0_f32; 5760 * count];
    let mut last_granule = None;
    // The packet after the header holds the tags.
    let mut tags_seen = false;
    while let Some(packet) = reader.read_packet().map_err(|e| fail(e.to_string()))? {
        if !tags_seen {
            tags_seen = true;
            continue;
        }
        // SAFETY: `pcm` has room for the longest Opus frame (120 ms) per channel.
        let n = unsafe { opus_decode_float(decoder, packet.data.as_ptr(), packet.data.len() as i32, pcm.as_mut_ptr(), 5760, 0) };
        if n > 0 {
            interleaved.extend_from_slice(&pcm[..n as usize * count]);
        }
        if packet.last_in_page() {
            last_granule = Some(packet.absgp_page() as usize);
        }
    }
    let decoded = interleaved.len() / count;
    let end = last_granule.map_or(decoded, |g| g.min(decoded)).max(pre_skip);
    let channels = (0..count).map(|c| (pre_skip..end).map(|i| interleaved[i * count + c]).collect()).collect();
    Ok(AudioBuffer::new(48_000, channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> AudioBuffer {
        let ch: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        AudioBuffer::new(48_000, vec![ch.clone(), ch])
    }

    fn long_tone(rate: u32) -> AudioBuffer {
        let ch: Vec<f32> = (0..rate as usize).map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin() * 0.5).collect();
        AudioBuffer::new(rate, vec![ch.clone(), ch])
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("soniq-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn roundtrip(name: &str, encoding: &Encoding, buffer: &AudioBuffer) -> AudioBuffer {
        let path = temp(name);
        write_export(&path, buffer, encoding).unwrap();
        let decoded = crate::decode_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        decoded
    }

    fn with_bits(format: ExportFormat, bits: u16) -> Encoding {
        Encoding { bits, ..Encoding::new(format) }
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn flac_block_size_fits_the_length() {
        assert_eq!(block_size_for(4800), 2400);
        assert_eq!(block_size_for(48_000 * 180), 4608);
        // A prime length still leaves almost no padding.
        let frames = 1_000_003;
        let bs = block_size_for(frames);
        assert!((bs - frames % bs) % bs < 64, "padding with {bs}");
    }

    #[test]
    fn integer_wav_and_flac_decode_back_within_dither() {
        for (name, format, bits) in [("t16.wav", ExportFormat::Wav, 16), ("t24.wav", ExportFormat::Wav, 24), ("t.flac", ExportFormat::Flac, 24)] {
            let decoded = roundtrip(name, &with_bits(format, bits), &tone());
            assert_eq!(decoded.frames(), 4800, "{name}");
            assert_eq!(decoded.sample_rate, 48_000, "{name}");
            let original = tone();
            let worst = decoded.channels[0].iter().zip(&original.channels[0]).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
            assert!(worst < 2.0 / 32_768.0 * 2.0, "{name}: error {worst}");
        }
    }

    #[test]
    fn tags_do_not_break_wav_or_flac() {
        let metadata = Metadata { title: "Night Drive".into(), artist: "Soniq".into(), ..Default::default() };
        for (name, format) in [("tagged.wav", ExportFormat::Wav), ("tagged.flac", ExportFormat::Flac)] {
            let encoding = Encoding { metadata: metadata.clone(), ..with_bits(format, 16) };
            let path = temp(name);
            write_export(&path, &tone(), &encoding).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let decoded = crate::decode_file(&path).unwrap();
            std::fs::remove_file(&path).ok();
            assert_eq!(decoded.frames(), 4800, "{name}");
            assert!(bytes.windows(11).any(|w| w == b"Night Drive"), "{name} carries the title");
        }
    }

    #[test]
    fn chapters_go_into_every_format() {
        let chapters = vec![
            Chapter { title: "Intro".into(), start_ms: 0, end_ms: 400 },
            Chapter { title: "The interview".into(), start_ms: 400, end_ms: 1000 },
        ];
        let metadata = Metadata { chapters, ..Default::default() };
        let input = long_tone(44_100);
        for (name, format) in [("ch.mp3", ExportFormat::Mp3), ("ch.wav", ExportFormat::Wav), ("ch.flac", ExportFormat::Flac), ("ch.ogg", ExportFormat::Ogg)] {
            let encoding = Encoding { metadata: metadata.clone(), ..with_bits(format, 16) };
            let path = temp(name);
            write_export(&path, &input, &encoding).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(crate::decode_file(&path).is_ok(), "{name} still decodes");
            if format == ExportFormat::Mp3 {
                let tag = id3::Tag::read_from_path(&path).unwrap();
                let found: Vec<(u32, u32)> = tag.chapters().map(|c| (c.start_time, c.end_time)).collect();
                assert_eq!(found, [(0, 400), (400, 1000)]);
                assert_eq!(tag.tables_of_contents().next().map(|t| t.elements.len()), Some(2));
            } else if format == ExportFormat::Wav {
                assert!(bytes.windows(4).any(|w| w == b"cue "), "WAV has cue points");
            } else {
                assert!(bytes.windows(19).any(|w| w == b"CHAPTER002=00:00:00"), "{name} has chapter comments");
            }
            assert!(bytes.windows(13).any(|w| w == b"The interview"), "{name} names the chapter");
            std::fs::remove_file(&path).ok();
        }
        assert_eq!(chapter_time(3_723_004), "01:02:03.004");
    }

    #[test]
    fn lossy_formats_decode_back_to_the_same_tone() {
        let input = long_tone(44_100);
        for (name, format) in [("t.mp3", ExportFormat::Mp3), ("t.ogg", ExportFormat::Ogg)] {
            let decoded = roundtrip(name, &Encoding::new(format), &input);
            assert_eq!(decoded.sample_rate, 44_100, "{name}");
            // Encoder delay and padding add a little at the ends.
            assert!(decoded.frames() >= 44_100 && decoded.frames() < 44_100 + 4096, "{name}: {}", decoded.frames());
            let level = rms(&decoded.channels[0][10_000..30_000]);
            assert!((level - 0.3536).abs() < 0.02, "{name}: rms {level}");
        }
    }

    #[test]
    fn opus_round_trips_with_the_exact_length() {
        let input = long_tone(48_000);
        let path = temp("t.opus");
        let encoding = Encoding { metadata: Metadata { title: "Opus test".into(), ..Default::default() }, ..Encoding::new(ExportFormat::Opus) };
        write_export(&path, &input, &encoding).unwrap();
        let decoded = decode_opus(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(decoded.frames(), 48_000);
        let level = rms(&decoded.channels[1][10_000..30_000]);
        assert!((level - 0.3536).abs() < 0.02, "rms {level}");
    }
}
