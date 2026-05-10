use log::{debug, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Alibaba Cloud DashScope (Qwen) API endpoint.
const DASHSCOPE_LLM_URL: &str =
    "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";

/// System prompt template for translation. `{target_language}` is replaced at runtime.
const TRANSLATE_SYSTEM_PROMPT_TEMPLATE: &str =
    "你是一个翻译助手。将用户输入的文本翻译成{target_language}。只返回翻译结果，不要添加任何解释。保持原文的语气和风格。";

/// System prompt for speak-to-edit: modify selected text according to a voice command.
const EDIT_TEXT_SYSTEM_PROMPT: &str =
    "你是一个文本编辑助手。用户会提供一段已选中的文本和一个语音指令。请根据指令修改文本。只返回修改后的文本，不要添加任何解释。";

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
    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Polish the raw ASR text using Qwen LLM.
    ///
    /// If polishing is disabled, returns the raw text unchanged.
    /// If the API call fails, falls back to the raw text.
    /// The optional `dictionary` slice lists custom terms that must be preserved verbatim.
    /// The optional `tone` string adjusts the stylistic tone of the output (e.g. "formal,
    /// professional"). When empty the default prompt tone is used.
    pub async fn polish(
        &self,
        raw_text: &str,
        dictionary: &[String],
        tone: &str,
    ) -> Result<String, LlmError> {
        if raw_text.is_empty() {
            debug!("LLM polish: empty input, skipping");
            return Ok(String::new());
        }

        if !self.enabled {
            debug!("LLM polish: disabled, returning raw text");
            return Ok(raw_text.to_string());
        }

        info!("LLM polish request (tone=\"{}\"): \"{}\"", tone, raw_text);

        // Build system prompt, appending dictionary terms and tone instructions when provided.
        let mut system_prompt = if dictionary.is_empty() {
            POLISH_SYSTEM_PROMPT.to_string()
        } else {
            let terms = dictionary.join("、");
            format!(
                "{}\n\n以下专有名词必须原样保留，不要修改：{}",
                POLISH_SYSTEM_PROMPT, terms
            )
        };

        // Append tone instruction when a non-empty tone descriptor is given.
        if !tone.is_empty() {
            debug!("LLM polish: applying tone \"{}\"", tone);
            system_prompt.push_str(&format!("\n\n请使用{}的语气风格。", tone));
        }

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

    /// Edit `selected_text` according to `voice_command` using Qwen LLM.
    ///
    /// Used by the speak-to-edit feature. Returns the modified text, or an error
    /// if the API call fails. On error the caller should fall back to leaving the
    /// original selection unchanged.
    pub async fn edit_text(
        &self,
        selected_text: &str,
        voice_command: &str,
    ) -> Result<String, LlmError> {
        if selected_text.is_empty() {
            debug!("LLM edit_text: empty selected text, skipping");
            return Ok(String::new());
        }
        if voice_command.is_empty() {
            debug!("LLM edit_text: empty voice command, returning selected text unchanged");
            return Ok(selected_text.to_string());
        }

        info!(
            "LLM edit_text request: command=\"{}\", text_len={}",
            voice_command,
            selected_text.len()
        );

        let user_message = format!(
            "选中的文本：\n{selected_text}\n\n指令：{voice_command}"
        );

        let request_body = ChatRequest {
            model: "qwen-plus".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: EDIT_TEXT_SYSTEM_PROMPT.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user_message,
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
        debug!("LLM edit_text response status: {}", status);

        let body = response
            .text()
            .await
            .map_err(|e| LlmError::NetworkError(format!("Failed to read response: {e}")))?;

        if !status.is_success() {
            warn!("LLM edit_text API error: HTTP {status}, body: {body}");
            return Err(LlmError::ApiError(format!("HTTP {status}: {body}")));
        }

        debug!("LLM edit_text response body: {}", body);

        let chat_response: ChatResponse = serde_json::from_str(&body)
            .map_err(|e| LlmError::ParseError(format!("Failed to parse response: {e}")))?;

        let edited = chat_response
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_else(|| selected_text.to_string());

        info!(
            "LLM edit_text result: \"{}\" -> \"{}\"",
            &selected_text[..selected_text.len().min(60)],
            &edited[..edited.len().min(60)]
        );

        Ok(edited)
    }

    /// Translate `text` into `target_language` using Qwen LLM.
    ///
    /// Returns the translated string, or an error if the API call fails.
    pub async fn translate(&self, text: &str, target_language: &str) -> Result<String, LlmError> {
        if text.is_empty() {
            debug!("LLM translate: empty input, skipping");
            return Ok(String::new());
        }

        info!("LLM translate request: \"{}\" -> {}", text, target_language);

        let system_prompt = TRANSLATE_SYSTEM_PROMPT_TEMPLATE
            .replace("{target_language}", target_language);

        let request_body = ChatRequest {
            model: "qwen-plus".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: text.to_string(),
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
        debug!("LLM translate response status: {}", status);

        let body = response
            .text()
            .await
            .map_err(|e| LlmError::NetworkError(format!("Failed to read response: {e}")))?;

        if !status.is_success() {
            warn!("LLM translate API error: HTTP {status}, body: {body}");
            return Err(LlmError::ApiError(format!("HTTP {status}: {body}")));
        }

        debug!("LLM translate response body: {}", body);

        let chat_response: ChatResponse = serde_json::from_str(&body)
            .map_err(|e| LlmError::ParseError(format!("Failed to parse response: {e}")))?;

        let translated = chat_response
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_else(|| text.to_string());

        Ok(translated)
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
#[allow(clippy::enum_variant_names)]
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
        let result = client.polish("", &[], "").await.unwrap();
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_polish_disabled() {
        let mut client = LlmClient::new("test-key".to_string());
        client.set_enabled(false);
        let result = client.polish("嗯那个你好", &[], "").await.unwrap();
        assert_eq!(result, "嗯那个你好");
    }

    #[tokio::test]
    async fn test_polish_disabled_with_dictionary() {
        let mut client = LlmClient::new("test-key".to_string());
        client.set_enabled(false);
        let dict = vec!["Tauri".to_string(), "Rust".to_string()];
        let result = client.polish("嗯那个你好", &dict, "").await.unwrap();
        assert_eq!(result, "嗯那个你好");
    }

    #[tokio::test]
    async fn test_polish_disabled_with_tone() {
        // Tone parameter should not prevent early-return when polishing is disabled.
        let mut client = LlmClient::new("test-key".to_string());
        client.set_enabled(false);
        let result = client
            .polish("嗯那个你好", &[], "formal, professional")
            .await
            .unwrap();
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
