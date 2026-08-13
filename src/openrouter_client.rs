use super::{
    ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse, ModelInfo,
};

/// Truncate a string to `max_len` characters, appending "..." if truncated.
/// Whitespace is trimmed first so binary/whitespace-heavy bodies don't
/// produce empty log lines.
pub(crate) fn truncate_str(s: &str, max_len: usize) -> String {
    let trimmed = s.trim();
    let char_count = trimmed.chars().count();
    if char_count <= max_len {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed.chars().take(max_len).collect::<String>())
    }
}

pub struct OpenRouterClient {
    api_key: String,
    http_client: reqwest::Client,
    base_url: String,
}

impl OpenRouterClient {
    pub fn new(api_key: String) -> Self {
        Self::new_with_base_url(api_key, "https://openrouter.ai/api/v1")
    }

    pub(crate) fn new_with_base_url(api_key: String, base_url: &str) -> Self {
        Self {
            api_key,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build reqwest client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetch available models and their context lengths from OpenRouter.
    pub async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let response = self
            .http_client
            .get(format!("{}/models", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|e| format!("(failed to read body: {})", e));
        if !status.is_success() {
            anyhow::bail!("OpenRouter API error ({}): {}", status, body_text);
        }

        #[derive(serde::Deserialize)]
        struct ModelsApiResponse {
            data: Vec<ModelInfo>,
        }

        let resp: ModelsApiResponse = serde_json::from_str(&body_text).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse models response (status {}): {}. Body: {}",
                status,
                e,
                truncate_str(&body_text, 500)
            )
        })?;
        Ok(resp.data)
    }

    /// Generate embeddings for a text string using the given model.
    pub async fn embeddings(&self, model: &str, input: &str) -> anyhow::Result<Vec<f32>> {
        let response = self
            .http_client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&EmbeddingRequest {
                model: model.to_string(),
                input: input.to_string(),
            })
            .send()
            .await?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|e| format!("(failed to read body: {})", e));
        if !status.is_success() {
            anyhow::bail!(
                "OpenRouter embeddings API error ({}): {}",
                status,
                body_text
            );
        }

        let resp: EmbeddingResponse = serde_json::from_str(&body_text).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse embeddings response (status {}): {}. Body: {}",
                status,
                e,
                truncate_str(&body_text, 500)
            )
        })?;
        resp.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| anyhow::anyhow!("No embedding data in response"))
    }

    /// Send a chat completion request to OpenRouter. A 200 response whose body is
    /// truncated or cannot be decoded is retried once because no completion was
    /// delivered and therefore replaying the request is safe.
    pub async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> anyhow::Result<ChatCompletionResponse> {
        const MAX_ATTEMPTS: usize = 2;

        for attempt in 1..=MAX_ATTEMPTS {
            match self.chat_completion_once(request).await {
                Ok(completion) => return Ok(completion),
                Err(error) if attempt < MAX_ATTEMPTS && error.retryable => {
                    log::warn!(
                        "OpenRouter chat response could not be decoded (attempt {attempt}/{MAX_ATTEMPTS}); retrying once: {}",
                        error.message
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(error) => return Err(anyhow::anyhow!(error.message)),
            }
        }
        unreachable!("chat completion retry loop always returns")
    }

    async fn chat_completion_once(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, ChatCompletionError> {
        let response = self
            .http_client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(request)
            .send()
            .await
            .map_err(|error| ChatCompletionError::new(error.to_string(), false))?;

        let status = response.status();
        let body_text = match response.text().await {
            Ok(body) => body,
            Err(error) => {
                return Err(ChatCompletionError::new(
                    format!(
                        "Failed to read chat completion response body (status {}): {}",
                        status, error
                    ),
                    status.is_success() && (error.is_decode() || error.is_body()),
                ));
            }
        };
        if !status.is_success() {
            return Err(ChatCompletionError::new(
                format!("OpenRouter API error ({}): {}", status, body_text),
                false,
            ));
        }

        let completion: ChatCompletionResponse = serde_json::from_str(&body_text).map_err(|e| {
            ChatCompletionError::new(
                format!(
                    "Failed to parse chat completion response (status {}): {}. Body: {}",
                    status,
                    e,
                    truncate_str(&body_text, 500)
                ),
                body_text.trim().is_empty(),
            )
        })?;
        if completion.choices.is_empty() {
            log::warn!(
                "OpenRouter returned 200 OK but with empty choices (likely a provider error wrapped by OpenRouter). Body: {}",
                truncate_str(&body_text, 500)
            );
        }
        Ok(completion)
    }
}

struct ChatCompletionError {
    message: String,
    retryable: bool,
}

impl ChatCompletionError {
    fn new(message: String, retryable: bool) -> Self {
        Self { message, retryable }
    }
}
