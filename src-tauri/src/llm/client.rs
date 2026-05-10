use log::{debug, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Alibaba Cloud DashScope (Qwen) API endpoint.
const DASHSCOPE_LLM_URL: &str =
    "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";

/// System prompt for text polishing.
const POLISH_SYSTEM_PROMPT: &str = r#"你是一个语音转文字的后处理助手。你的任务是润色语音识别的原始文本，使其更加通顺自然。

规则：
1. 去除口头禅和语气词（嗯、啊、那个、就是说、然后等）
2. 修正明显的语音识别错误
3. 添加合适的标点符号
4. 保持原文的意思不变，不要添加或删除实质内容
5. 如果原文已经很好，直接返回原文
6. 只返回润色后的文本，不要添加任何解释

示例：
输入：嗯那个我想说的是就是说今天天气还不错啊我们可以出去走走
输出：我想说的是，今天天气还不错，我们可以出去走走。"#;

/// Client for Alibaba Cloud Qwen LLM text polishing service.
#[derive(Clone)]
pub struct LlmClient {
    api_key: String,
    http: Client,
    /// Whether polishing is enabled. When disabled, raw text is returned as-is.
    enabled: bool,
}

impl LlmClient {
    /// Create a new LLM client with the given DashScope API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: Client::new(),
            enabled: true,
        }
    }

    /// Enable or disable text polishing.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if polishing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Polish the raw ASR text using Qwen LLM.
    ///
    /// If polishing is disabled, returns the raw text unchanged.
    /// If the API call fails, falls back to the raw text.
    /// The optional `dictionary` slice lists custom terms that must be preserved verbatim.
    pub async fn polish(&self, raw_text: &str, dictionary: &[String]) -> Result<String, LlmError> {
        if raw_text.is_empty() {
            debug!("LLM polish: empty input, skipping");
            return Ok(String::new());
        }

        if !self.enabled {
            debug!("LLM polish: disabled, returning raw text");
            return Ok(raw_text.to_string());
        }

        info!("LLM polish request: \"{}\"", raw_text);

        // Build system prompt, appending dictionary terms when provided.
        let system_prompt = if dictionary.is_empty() {
            POLISH_SYSTEM_PROMPT.to_string()
        } else {
            let terms = dictionary.join("、");
            format!(
                "{}\n\n以下专有名词必须原样保留，不要修改：{}",
                POLISH_SYSTEM_PROMPT, terms
            )
        };

        let request_body = ChatRequest {
            model: "qwen-plus".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: raw_text.to_string(),
                },
            ],
            max_tokens: 2048,
            temperature: 0.3,
        };

        let response = self
            .http
            .post(DASHSCOPE_LLM_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError(format!("{e}")))?;

        let status = response.status();
        debug!("LLM response status: {}", status);

        let body = response
            .text()
            .await
            .map_err(|e| LlmError::NetworkError(format!("Failed to read response: {e}")))?;

        if !status.is_success() {
            warn!("LLM API error: HTTP {status}, body: {body}");
            return Err(LlmError::ApiError(format!("HTTP {status}: {body}")));
        }

        debug!("LLM response body: {}", body);

        let chat_response: ChatResponse = serde_json::from_str(&body)
            .map_err(|e| LlmError::ParseError(format!("Failed to parse response: {e}")))?;

        let polished = chat_response
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_else(|| raw_text.to_string());

        Ok(polished)
    }
}

// --- Request types (OpenAI-compatible format) ---

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

// --- Response types ---

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

/// Errors that can occur during LLM polishing.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Failed to parse API response: {0}")]
    ParseError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_client_creation() {
        let client = LlmClient::new("test-key".to_string());
        assert!(client.is_enabled());
    }

    #[test]
    fn test_set_enabled() {
        let mut client = LlmClient::new("test-key".to_string());
        client.set_enabled(false);
        assert!(!client.is_enabled());
    }

    #[tokio::test]
    async fn test_polish_empty_string() {
        let client = LlmClient::new("test-key".to_string());
        let result = client.polish("", &[]).await.unwrap();
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_polish_disabled() {
        let mut client = LlmClient::new("test-key".to_string());
        client.set_enabled(false);
        let result = client.polish("嗯那个你好", &[]).await.unwrap();
        assert_eq!(result, "嗯那个你好");
    }

    #[tokio::test]
    async fn test_polish_disabled_with_dictionary() {
        let mut client = LlmClient::new("test-key".to_string());
        client.set_enabled(false);
        let dict = vec!["Tauri".to_string(), "Rust".to_string()];
        let result = client.polish("嗯那个你好", &dict).await.unwrap();
        assert_eq!(result, "嗯那个你好");
    }

    #[test]
    fn test_parse_chat_response() {
        let json = r#"{
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "今天天气不错，我们出去走走。"
                    }
                }
            ]
        }"#;
        let response: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            response.choices[0].message.content,
            "今天天气不错，我们出去走走。"
        );
    }
}
