use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use std::sync::{Arc, Mutex};

use super::wav::encode_wav;

/// Target sample rate for ASR.
const TARGET_SAMPLE_RATE: u32 = 16000;

/// Shared recording buffer that can be passed to the audio callback.
#[derive(Clone, Default)]
pub struct RecordingBuffer {
    inner: Arc<Mutex<RecordingInner>>,
}

#[derive(Default)]
struct RecordingInner {
    samples: Vec<f32>,
    is_recording: bool,
}

impl RecordingBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark recording as active and clear previous samples.
    pub fn start(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.samples.clear();
        inner.is_recording = true;
    }

    /// Mark recording as stopped.
    pub fn stop(&self) {
        self.inner.lock().unwrap().is_recording = false;
    }

    /// Check if currently recording.
    pub fn is_recording(&self) -> bool {
        self.inner.lock().unwrap().is_recording
    }

    /// Get a copy of recorded samples.
    pub fn samples(&self) -> Vec<f32> {
        self.inner.lock().unwrap().samples.clone()
    }

    /// Push samples into the buffer (called from audio callback).
    fn push_samples(&self, data: &[f32]) {
        let mut inner = self.inner.lock().unwrap();
        if inner.is_recording {
            inner.samples.extend_from_slice(data);
        }
    }
}

/// Cross-platform audio recorder that captures microphone input.
///
/// The recorder is NOT Send/Sync due to cpal::Stream. It must be
/// created and used on the same thread, or managed via a dedicated
/// audio thread.
pub struct AudioRecorder {
    buffer: RecordingBuffer,
    _stream: Stream,
    device_sample_rate: u32,
    device_channels: u16,
}

impl AudioRecorder {
    /// Create a new `AudioRecorder` using the default input device.
    pub fn new(buffer: RecordingBuffer) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(AudioError::NoInputDevice)?;

        let config = device.default_input_config().map_err(|e| {
            AudioError::ConfigError(format!("Failed to get default input config: {e}"))
        })?;

        let device_sample_rate = config.sample_rate().0;
        let device_channels = config.channels();

        let sample_format = config.sample_format();
        let stream_config: StreamConfig = config.into();

        let buf = buffer.clone();
        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    buf.push_samples(data);
                },
                |err| eprintln!("Audio stream error: {err}"),
                None,
            ),
            SampleFormat::I16 => {
                let buf2 = buffer.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let float_samples: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        buf2.push_samples(&float_samples);
                    },
                    |err| eprintln!("Audio stream error: {err}"),
                    None,
                )
            }
            _ => {
                return Err(AudioError::ConfigError(format!(
                    "Unsupported sample format: {sample_format:?}"
                )));
            }
        }
        .map_err(|e| AudioError::StreamError(format!("Failed to build input stream: {e}")))?;

        // Start the stream immediately; recording is controlled by the buffer flag.
        stream
            .play()
            .map_err(|e| AudioError::StreamError(format!("Failed to start stream: {e}")))?;

        Ok(Self {
            buffer,
            _stream: stream,
            device_sample_rate,
            device_channels,
        })
    }

    /// Get the recording buffer.
    pub fn buffer(&self) -> &RecordingBuffer {
        &self.buffer
    }

    /// Get the recorded audio as 16kHz mono WAV bytes.
    pub fn get_wav_data(&self) -> Result<Vec<u8>, AudioError> {
        let samples = self.buffer.samples();
        if samples.is_empty() {
            return Err(AudioError::NoData);
        }

        // Mix to mono if multi-channel
        let mono_samples = if self.device_channels > 1 {
            mix_to_mono(&samples, self.device_channels)
        } else {
            samples
        };

        // Resample to target sample rate if needed
        let resampled = if self.device_sample_rate != TARGET_SAMPLE_RATE {
            resample(&mono_samples, self.device_sample_rate, TARGET_SAMPLE_RATE)
        } else {
            mono_samples
        };

        encode_wav(&resampled, TARGET_SAMPLE_RATE)
            .map_err(|e| AudioError::EncodingError(format!("WAV encoding failed: {e}")))
    }
}

/// Mix multi-channel audio to mono by averaging channels.
fn mix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels as usize;
    samples
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Simple linear interpolation resampler.
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = src_pos - src_idx as f64;

        let sample = if src_idx + 1 < samples.len() {
            samples[src_idx] * (1.0 - frac as f32) + samples[src_idx + 1] * frac as f32
        } else if src_idx < samples.len() {
            samples[src_idx]
        } else {
            0.0
        };

        output.push(sample);
    }

    output
}

/// Errors that can occur during audio recording.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("No input device found")]
    NoInputDevice,

    #[error("Audio configuration error: {0}")]
    ConfigError(String),

    #[error("Audio stream error: {0}")]
    StreamError(String),

    #[error("No audio data recorded")]
    NoData,

    #[error("Audio encoding error: {0}")]
    EncodingError(String),
}
