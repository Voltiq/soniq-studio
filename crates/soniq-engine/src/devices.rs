//! Audio device discovery and stream setup across every host API cpal supports.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{BufferSize, FromSample, SampleFormat, SampleRate, SizedSample, SupportedBufferSize};
use serde::{Deserialize, Serialize};

use crate::render::{Links, Renderer};
use crate::transport::Transport;
use crate::EngineStatus;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Input,
    Output,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub host: String,
    pub name: String,
    pub direction: Direction,
    pub is_default: bool,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
}

/// What the settings dialog may offer for one output device.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOptions {
    pub sample_rates: Vec<u32>,
    pub default_sample_rate: u32,
    pub buffer_sizes: Vec<u32>,
}

/// The user's choice. `None` everywhere means "system default".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineConfig {
    pub host: Option<String>,
    pub output: Option<String>,
    pub input: Option<String>,
    pub sample_rate: Option<u32>,
    pub buffer_frames: Option<u32>,
    /// Added to the measured round-trip latency when placing recordings, in
    /// ms. Drivers do not always report every buffer they add.
    pub record_offset_ms: f64,
    /// Cores to spread the mix over; `None` uses all but one (up to eight).
    pub cores: Option<u32>,
    /// The safety limiter before the speakers is switched off.
    pub safety_limiter_off: bool,
}

const COMMON_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
const COMMON_BUFFERS: [u32; 7] = [32, 64, 128, 256, 512, 1024, 2048];

pub fn list_devices() -> Vec<DeviceInfo> {
    let mut devices = Vec::new();
    for host_id in cpal::available_hosts() {
        let Ok(host) = cpal::host_from_id(host_id) else {
            continue;
        };
        let host_name = host_id.name();

        let default_out = host.default_output_device().and_then(|d| d.name().ok());
        if let Ok(outputs) = host.output_devices() {
            for device in outputs {
                let config = device.default_output_config().ok();
                devices.push(describe(host_name, &device, Direction::Output, default_out.as_deref(), config));
            }
        }

        let default_in = host.default_input_device().and_then(|d| d.name().ok());
        if let Ok(inputs) = host.input_devices() {
            for device in inputs {
                let config = device.default_input_config().ok();
                devices.push(describe(host_name, &device, Direction::Input, default_in.as_deref(), config));
            }
        }
    }
    devices
}

fn describe(
    host: &str,
    device: &cpal::Device,
    direction: Direction,
    default_name: Option<&str>,
    config: Option<cpal::SupportedStreamConfig>,
) -> DeviceInfo {
    let name = device.name().unwrap_or_else(|_| "Unknown device".into());
    DeviceInfo {
        host: host.to_string(),
        is_default: default_name == Some(name.as_str()),
        sample_rate: config.as_ref().map(|c| c.sample_rate().0),
        channels: config.as_ref().map(|c| c.channels()),
        direction,
        name,
    }
}

pub(crate) fn find_input(host: &cpal::Host, name: Option<&str>) -> Option<cpal::Device> {
    name.and_then(|name| host.input_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(name)))
        .or_else(|| host.default_input_device())
}

pub(crate) fn find_host(name: Option<&str>) -> cpal::Host {
    name.and_then(|name| cpal::available_hosts().into_iter().find(|id| id.name() == name))
        .and_then(|id| cpal::host_from_id(id).ok())
        .unwrap_or_else(cpal::default_host)
}

/// The output the engine would open for `config` right now: the chosen
/// device if it is there, else the system default.
pub fn output_device_name(config: &EngineConfig) -> Option<String> {
    let host = find_host(config.host.as_deref());
    find_output(&host, config.output.as_deref())?.name().ok()
}

fn find_output(host: &cpal::Host, name: Option<&str>) -> Option<cpal::Device> {
    name.and_then(|name| host.output_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(name)))
        .or_else(|| host.default_output_device())
}

pub fn device_options(host: Option<&str>, output: Option<&str>) -> Result<DeviceOptions, String> {
    let host = find_host(host);
    let device = find_output(&host, output).ok_or("No audio output device was found.")?;
    let default = device.default_output_config().map_err(|e| e.to_string())?;
    let ranges: Vec<_> = device.supported_output_configs().map_err(|e| e.to_string())?.collect();

    let mut sample_rates: Vec<u32> = COMMON_RATES
        .into_iter()
        .filter(|r| ranges.iter().any(|c| c.min_sample_rate().0 <= *r && *r <= c.max_sample_rate().0))
        .collect();
    if !sample_rates.contains(&default.sample_rate().0) {
        sample_rates.push(default.sample_rate().0);
        sample_rates.sort_unstable();
    }

    let buffer_sizes = match default.buffer_size() {
        SupportedBufferSize::Range { min, max } => COMMON_BUFFERS.into_iter().filter(|b| *min <= *b && *b <= *max).collect(),
        SupportedBufferSize::Unknown => Vec::new(),
    };

    Ok(DeviceOptions { sample_rates, default_sample_rate: default.sample_rate().0, buffer_sizes })
}

pub(crate) struct StreamParts {
    pub transport: Arc<Transport>,
    pub buffer_frames: Arc<AtomicU32>,
    /// From rendering a sample to hearing it, in µs, as the driver reports it.
    pub output_latency_us: Arc<AtomicU32>,
    pub links: Links,
    /// Counts callbacks: if it stops moving, the device has gone quiet.
    pub alive: Arc<AtomicU64>,
    /// The driver said the device is gone.
    pub lost: Arc<AtomicBool>,
}

/// Opens the configured output. Settings the device does not support fall
/// back to its defaults. Must run on the thread that will own the stream.
pub(crate) fn open_output(config: &EngineConfig, parts: StreamParts) -> Result<(cpal::Stream, EngineStatus), String> {
    let host = find_host(config.host.as_deref());
    let device = find_output(&host, config.output.as_deref()).ok_or("No audio output device was found.")?;
    let default = device.default_output_config().map_err(|e| format!("Could not read the output format: {e}"))?;

    let mut format = default.sample_format();
    let mut stream_config: cpal::StreamConfig = default.config();
    if let Some(rate) = config.sample_rate.filter(|r| *r != stream_config.sample_rate.0) {
        let supported = device.supported_output_configs().map_err(|e| e.to_string())?.find(|c| {
            c.channels() == stream_config.channels && c.min_sample_rate().0 <= rate && rate <= c.max_sample_rate().0
        });
        if let Some(range) = supported {
            format = range.sample_format();
            stream_config = range.with_sample_rate(SampleRate(rate)).config();
        }
    }
    stream_config.buffer_size = match (config.buffer_frames, default.buffer_size()) {
        (Some(n), SupportedBufferSize::Range { min, max }) if *min <= n && n <= *max => BufferSize::Fixed(n),
        _ => BufferSize::Default,
    };

    let rate = stream_config.sample_rate.0;
    parts.transport.set_sample_rate(rate);
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &stream_config, parts),
        SampleFormat::I16 => build::<i16>(&device, &stream_config, parts),
        SampleFormat::U16 => build::<u16>(&device, &stream_config, parts),
        SampleFormat::I32 => build::<i32>(&device, &stream_config, parts),
        other => Err(format!("Unsupported sample format: {other:?}")),
    }?;

    let status = EngineStatus {
        running: true,
        host: host.id().name().to_string(),
        device: device.name().unwrap_or_else(|_| "Default output".into()),
        sample_rate: rate,
        channels: stream_config.channels,
        buffer_frames: match stream_config.buffer_size {
            BufferSize::Fixed(n) => n,
            BufferSize::Default => 0,
        },
        output_latency_ms: 0.0,
        error: None,
        cores: 1,
    };
    Ok((stream, status))
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, parts: StreamParts) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    let buffer_frames = parts.buffer_frames;
    let latency = parts.output_latency_us;
    let (alive, lost) = (parts.alive, parts.lost);
    let mut renderer = Renderer::new(parts.transport, config.sample_rate.0, parts.links);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                alive.fetch_add(1, Ordering::Relaxed);
                buffer_frames.store((data.len() / channels) as u32, Ordering::Relaxed);
                let stamp = info.timestamp();
                if let Some(ahead) = stamp.playback.duration_since(&stamp.callback) {
                    latency.store(ahead.as_micros().min(u128::from(u32::MAX)) as u32, Ordering::Relaxed);
                }
                renderer.process(data, channels);
            },
            move |err| {
                eprintln!("audio stream error: {err}");
                if matches!(err, cpal::StreamError::DeviceNotAvailable) {
                    lost.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| format!("Could not open the audio stream: {e}"))
}
