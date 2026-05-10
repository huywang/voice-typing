use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Alibaba Cloud Paraformer ASR API endpoint.
const DASHSCOPE_ASR_URL: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription";

/// Client for Alibaba Cloud's Paraformer speech recognition service.
///
/// Uses the DashScope API to transcribe audio to text. Supports Chinese
/// and English recognition.
#[derive(Clone)]
pub struct AsrClient {
    api_key: String,
    http: Client,
}

impl AsrClient {
    /// Create a new ASR client with the given DashScope API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: Client::new(),
        }
    }

    /// Recognize speech from WAV audio data.
    ///
    /// Sends the audio to Alibaba Cloud Paraformer and returns the
    /// transcribed text.
    pub async fn recognize(&self, wav_data: &[u8]) -> Result<String, AsrError> {
        if wav_data.is_empty() {
            return Err(AsrError::EmptyAudio);
        }

        // Encode audio as base64 for the API request
        let audio_base64 = base64::engine::general_purpose::STANDARD.encode(wav_data);

        let request_body = AsrRequest {
            model: "paraformer-realtime-v2".to_string(),
            input: AsrInput {
                audio: audio_base64,
                format: "wav".to_string(),
                sample_rate: 16000,
            },
        };

        let response = self
            .http
            .post(DASHSCOPE_ASR_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AsrError::NetworkError(format!("{e}")))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AsrError::NetworkError(format!("Failed to read response: {e}")))?;

        if !status.is_success() {
            return Err(AsrError::ApiError(format!(
                "HTTP {status}: {body}"
            )));
        }

        let asr_response: AsrResponse = serde_json::from_str(&body)
            .map_err(|e| AsrError::ParseError(format!("Failed to parse response: {e}, body: {body}")))?;

        // Extract transcribed text from response
        let text = asr_response
            .output
            .and_then(|o| o.sentence)
            .and_then(|sentences| {
                Some(
                    sentences
                        .iter()
                        .map(|s| s.text.as_str())
                        .collect::<Vec<_>>()
                        .join(""),
                )
            })
            .unwrap_or_default();

        if text.is_empty() {
            return Err(AsrError::EmptyResult);
        }

        Ok(text)
    }
}

// --- Request types ---

#[derive(Debug, Serialize)]
struct AsrRequest {
    model: String,
    input: AsrInput,
}

#[derive(Debug, Serialize)]
struct AsrInput {
    audio: String,
    format: String,
    sample_rate: u32,
}

// --- Response types ---

#[derive(Debug, Deserialize)]
struct AsrResponse {
    output: Option<AsrOutput>,
}

#[derive(Debug, Deserialize)]
struct AsrOutput {
    sentence: Option<Vec<AsrSentence>>,
}

#[derive(Debug, Deserialize)]
struct AsrSentence {
    text: String,
}

/// Errors that can occur during ASR.
#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error("Empty audio data provided")]
    EmptyAudio,

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Failed to parse API response: {0}")]
    ParseError(String),

    #[error("ASR returned empty result")]
    EmptyResult,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asr_client_creation() {
        let client = AsrClient::new("test-key".to_string());
        assert_eq!(client.api_key, "test-key");
    }

    #[tokio::test]
    async fn test_recognize_empty_audio() {
        let client = AsrClient::new("test-key".to_string());
        let result = client.recognize(&[]).await;
        assert!(matches!(result, Err(AsrError::EmptyAudio)));
    }

    #[test]
    fn test_parse_asr_response() {
        let json = r#"{
            "output": {
                "sentence": [
                    {"text": "你好"},
                    {"text": "世界"}
                ]
            }
        }"#;
        let response: AsrResponse = serde_json::from_str(json).unwrap();
        let text: String = response
            .output
            .unwrap()
            .sentence
            .unwrap()
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(text, "你好世界");
    }

    #[test]
    fn test_parse_empty_asr_response() {
        let json = r#"{"output": null}"#;
        let response: AsrResponse = serde_json::from_str(json).unwrap();
        assert!(response.output.is_none());
    }
}
