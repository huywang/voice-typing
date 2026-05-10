use base64::Engine;
use futures_util::StreamExt;
use log::{debug, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Instant;

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
    /// Sends the audio as base64 to Qwen-ASR via the OpenAI-compatible API
    /// using SSE streaming. This reduces perceived latency by starting to
    /// process the response as soon as the first token arrives, rather than
    /// waiting for the full response.
    pub async fn recognize(&self, wav_data: &[u8]) -> Result<String, AsrError> {
        if wav_data.is_empty() {
            return Err(AsrError::EmptyAudio);
        }

        // Encode audio as base64 data URL
        let audio_base64 = base64::engine::general_purpose::STANDARD.encode(wav_data);
        let data_url = format!("data:audio/wav;base64,{audio_base64}");

        info!(
            "ASR request: {} bytes WAV -> {} bytes base64 (streaming)",
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
            stream: true,
        };

        let request_start = Instant::now();
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

        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|e| AsrError::NetworkError(format!("Failed to read error body: {e}")))?;
            warn!("ASR API error: HTTP {status}, body: {body}");
            return Err(AsrError::ApiError(format!("HTTP {status}: {body}")));
        }

        // Read SSE stream and accumulate text deltas.
        let mut full_text = String::new();
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut first_token_logged = false;

        while let Some(chunk) = byte_stream.next().await {
            let chunk =
                chunk.map_err(|e| AsrError::NetworkError(format!("Stream read error: {e}")))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process all complete lines in the buffer.
            loop {
                match buffer.find('\n') {
                    None => break,
                    Some(pos) => {
                        let line = buffer[..pos].trim().to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if line.starts_with("data: [DONE]") {
                            debug!("ASR SSE stream: received [DONE]");
                            break;
                        }

                        if let Some(json_str) = line.strip_prefix("data: ") {
                            match serde_json::from_str::<StreamChunk>(json_str) {
                                Ok(chunk_resp) => {
                                    if let Some(content) = chunk_resp
                                        .choices
                                        .first()
                                        .and_then(|c| c.delta.content.as_deref())
                                    {
                                        if !first_token_logged {
                                            info!(
                                                "ASR first token received in {:.1}s",
                                                request_start.elapsed().as_secs_f64()
                                            );
                                            first_token_logged = true;
                                        }
                                        full_text.push_str(content);
                                    }
                                }
                                Err(e) => {
                                    debug!("ASR SSE: failed to parse chunk (skipping): {e}, line: {line}");
                                }
                            }
                        }
                    }
                }
            }
        }

        let full_text = full_text.trim().to_string();

        if full_text.is_empty() {
            warn!("ASR streaming returned empty result");
            return Err(AsrError::EmptyResult);
        }

        info!(
            "ASR streaming completed in {:.1}s: \"{}\"",
            request_start.elapsed().as_secs_f64(),
            full_text
        );

        Ok(full_text)
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

// --- Non-streaming response types (kept for unit tests) ---

#[derive(Debug, Deserialize)]
#[cfg_attr(not(test), allow(dead_code))]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(not(test), allow(dead_code))]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(not(test), allow(dead_code))]
struct ChatResponseMessage {
    content: String,
}

// --- Streaming SSE response types ---

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
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
    #[allow(dead_code)]
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

    #[test]
    fn test_parse_stream_chunk() {
        let json = r#"{"choices":[{"delta":{"content":"你好"},"index":0}]}"#;
        let chunk: StreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(
            chunk.choices[0].delta.content.as_deref().unwrap(),
            "你好"
        );
    }

    #[test]
    fn test_parse_stream_chunk_empty_delta() {
        // Some chunks arrive with no content field (e.g. finish_reason chunks).
        let json = r#"{"choices":[{"delta":{},"index":0,"finish_reason":"stop"}]}"#;
        let chunk: StreamChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices[0].delta.content.is_none());
    }
}
