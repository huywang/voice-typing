use base64::Engine;
use log::{debug, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// DashScope OpenAI-compatible endpoint for Qwen-ASR.
const DASHSCOPE_ASR_URL: &str =
    "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";

/// Client for Alibaba Cloud Qwen-ASR speech recognition service.
///
/// Uses the DashScope OpenAI-compatible API with Qwen3-ASR-Flash model.
/// Supports Chinese and English recognition via base64 audio input.
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
    /// Sends the audio as base64 to Qwen-ASR via the OpenAI-compatible API.
    pub async fn recognize(&self, wav_data: &[u8]) -> Result<String, AsrError> {
        if wav_data.is_empty() {
            return Err(AsrError::EmptyAudio);
        }

        // Encode audio as base64 data URL
        let audio_base64 = base64::engine::general_purpose::STANDARD.encode(wav_data);
        let data_url = format!("data:audio/wav;base64,{audio_base64}");

        info!(
            "ASR request: {} bytes WAV -> {} bytes base64",
            wav_data.len(),
            audio_base64.len()
        );

        // qwen3-asr-flash supports automatic language detection for Chinese, English,
        // and mixed-language input. No language parameter needed.
        let request_body = ChatRequest {
            model: "qwen3-asr-flash".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: serde_json::json!([{
                    "type": "input_audio",
                    "input_audio": {
                        "data": data_url
                    }
                }]),
            }],
            stream: false,
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
        debug!("ASR response status: {}", status);

        let body = response
            .text()
            .await
            .map_err(|e| AsrError::NetworkError(format!("Failed to read response: {e}")))?;

        if !status.is_success() {
            warn!("ASR API error: HTTP {status}, body: {body}");
            return Err(AsrError::ApiError(format!("HTTP {status}: {body}")));
        }

        debug!("ASR response body: {}", body);

        let chat_response: ChatResponse = serde_json::from_str(&body).map_err(|e| {
            AsrError::ParseError(format!("Failed to parse response: {e}, body: {body}"))
        })?;

        let text = chat_response
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_default();

        if text.is_empty() {
            warn!("ASR returned empty result, response body: {body}");
            return Err(AsrError::EmptyResult);
        }

        Ok(text)
    }
}

// --- Request types (OpenAI-compatible) ---

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: serde_json::Value,
}

// --- Response types ---

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: String,
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
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "你好世界"
                    }
                }
            ]
        }"#;
        let response: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.choices[0].message.content, "你好世界");
    }

    #[test]
    fn test_parse_empty_response() {
        let json = r#"{"choices": []}"#;
        let response: ChatResponse = serde_json::from_str(json).unwrap();
        assert!(response.choices.is_empty());
    }
}
