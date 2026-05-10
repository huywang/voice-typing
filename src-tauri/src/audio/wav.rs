use std::io::Cursor;

/// Encode f32 PCM samples to WAV format bytes.
///
/// Output is 16-bit PCM WAV, mono, at the specified sample rate.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buffer = Cursor::new(Vec::new());
    let mut writer =
        hound::WavWriter::new(&mut buffer, spec).map_err(|e| format!("WAV writer error: {e}"))?;

    for &sample in samples {
        // Clamp and convert f32 [-1.0, 1.0] to i16
        let clamped = sample.clamp(-1.0, 1.0);
        let int_sample = (clamped * i16::MAX as f32) as i16;
        writer
            .write_sample(int_sample)
            .map_err(|e| format!("WAV write error: {e}"))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("WAV finalize error: {e}"))?;

    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_wav_basic() {
        // Generate 1 second of silence at 16kHz
        let samples = vec![0.0f32; 16000];
        let wav_data = encode_wav(&samples, 16000).unwrap();

        // WAV header is 44 bytes, each sample is 2 bytes (16-bit)
        assert_eq!(wav_data.len(), 44 + 16000 * 2);

        // Check RIFF header
        assert_eq!(&wav_data[0..4], b"RIFF");
        assert_eq!(&wav_data[8..12], b"WAVE");
    }

    #[test]
    fn test_encode_wav_sine_wave() {
        let sample_rate = 16000;
        let duration_secs = 0.1;
        let num_samples = (sample_rate as f32 * duration_secs) as usize;
        let freq = 440.0; // A4 note

        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect();

        let wav_data = encode_wav(&samples, sample_rate).unwrap();
        assert_eq!(wav_data.len(), 44 + num_samples * 2);
    }

    #[test]
    fn test_encode_wav_clamps_values() {
        // Values outside [-1.0, 1.0] should be clamped
        let samples = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        let wav_data = encode_wav(&samples, 16000).unwrap();
        assert_eq!(wav_data.len(), 44 + 5 * 2);
    }

    #[test]
    fn test_encode_wav_empty() {
        let samples: Vec<f32> = vec![];
        let wav_data = encode_wav(&samples, 16000).unwrap();
        // Just header, no sample data
        assert_eq!(wav_data.len(), 44);
    }
}
