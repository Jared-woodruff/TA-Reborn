//! Audio device output through cpal.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig, SupportedBufferSize};

use crate::AudioError;
use crate::mixer::Mixer;

/// Describes the opened output device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputInfo {
    pub host: String,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    /// Requested device buffer size in frames (`None` = backend default).
    pub buffer_frames: Option<u32>,
}

impl std::fmt::Display for OutputInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let OutputInfo { host, device, sample_rate, channels, sample_format, .. } = self;
        write!(f, "{device} via {host} ({sample_rate} Hz, {channels} ch, {sample_format})")?;
        if let Some(n) = self.buffer_frames {
            write!(f, ", {n}-frame buffer")?;
        }
        Ok(())
    }
}

/// A running output stream. Dropping it stops playback (and drops the mixer).
pub(crate) struct DeviceOutput {
    _stream: cpal::Stream,
    pub info: OutputInfo,
    /// Set by the stream's error callback when the device is lost and must be re-opened.
    pub failed: Arc<AtomicBool>,
}

/// Opens the default output device. `make_mixer` receives the device sample rate.
pub(crate) fn open(
    buffer_frames: Option<u32>,
    make_mixer: impl FnOnce(u32) -> Mixer,
) -> Result<DeviceOutput, AudioError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
    let device_name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "default device".into());
    let supported = device.default_output_config().map_err(|e| AudioError::Device(e.to_string()))?;
    let format = supported.sample_format();
    let mut config: StreamConfig = supported.config();
    let mut requested = None;
    if let Some(frames) = buffer_frames {
        let fits = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max } => (*min..=*max).contains(&frames),
            SupportedBufferSize::Unknown => true,
        };
        if fits {
            config.buffer_size = BufferSize::Fixed(frames);
            requested = Some(frames);
        }
    }
    let info = OutputInfo {
        host: host.id().name().to_string(),
        device: device_name,
        sample_rate: config.sample_rate,
        channels: config.channels,
        sample_format: format.to_string(),
        buffer_frames: requested,
    };
    let failed = Arc::new(AtomicBool::new(false));
    let mixer = make_mixer(config.sample_rate);
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, mixer, failed.clone()),
        SampleFormat::F64 => build::<f64>(&device, config, mixer, failed.clone()),
        SampleFormat::I16 => build::<i16>(&device, config, mixer, failed.clone()),
        SampleFormat::I24 => build::<cpal::I24>(&device, config, mixer, failed.clone()),
        SampleFormat::I32 => build::<i32>(&device, config, mixer, failed.clone()),
        SampleFormat::U16 => build::<u16>(&device, config, mixer, failed.clone()),
        SampleFormat::U8 => build::<u8>(&device, config, mixer, failed.clone()),
        SampleFormat::I8 => build::<i8>(&device, config, mixer, failed.clone()),
        other => return Err(AudioError::Device(format!("unsupported sample format {other}"))),
    }
    .map_err(|e| AudioError::Device(e.to_string()))?;
    stream.play().map_err(|e| AudioError::Device(e.to_string()))?;
    Ok(DeviceOutput { _stream: stream, info, failed })
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: StreamConfig,
    mut mixer: Mixer,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error> {
    let channels = config.channels as usize;
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| mixer.render(data, channels),
        move |err: cpal::Error| match err.kind() {
            ErrorKind::Xrun => log::debug!("audio buffer underrun"),
            ErrorKind::DeviceChanged => log::info!("audio output moved to another device"),
            ErrorKind::RealtimeDenied => log::info!("audio thread runs without real-time priority"),
            _ => {
                log::warn!("audio stream error: {err}");
                failed.store(true, Ordering::Release);
            }
        },
        None,
    )
}
