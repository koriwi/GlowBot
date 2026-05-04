use super::{
    ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse, ModelInfo,
};

/// Truncate a string to `max_len` characters, appending "..." if truncated.
pub(crate) fn truncate_str(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max_len).collect::<String>())
    }
}

pub struct OpenRouterClient {
    api_key: String,
    http_client: reqwest::Client,
}

impl OpenRouterClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http_client: reqwest::Client::new(),
        }
    }

    /// Fetch available models and their context lengths from OpenRouter.
    pub async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let response = self
            .http_client
            .get("https://openrouter.ai/api/v1/models")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        let status = response.status();
        let body_text = response.text().await.unwrap_or_else(|e| format!("(failed to read body: {})", e));
        if !status.is_success() {
            anyhow::bail!("OpenRouter API error ({}): {}", status, body_text);
        }

        #[derive(serde::Deserialize)]
        struct ModelsApiResponse {
            data: Vec<ModelInfo>,
        }

        let resp: ModelsApiResponse = serde_json::from_str(&body_text)
            .map_err(|e| anyhow::anyhow!("Failed to parse models response (status {}): {}. Body: {}", status, e, truncate_str(&body_text, 500)))?;
        Ok(resp.data)
    }

    /// Generate embeddings for a text string using the given model.
    pub async fn embeddings(&self, model: &str, input: &str) -> anyhow::Result<Vec<f32>> {
        let response = self
            .http_client
            .post("https://openrouter.ai/api/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&EmbeddingRequest {
                model: model.to_string(),
                input: input.to_string(),
            })
            .send()
            .await?;

        let status = response.status();
        let body_text = response.text().await.unwrap_or_else(|e| format!("(failed to read body: {})", e));
        if !status.is_success() {
            anyhow::bail!("OpenRouter embeddings API error ({}): {}", status, body_text);
        }

        let resp: EmbeddingResponse = serde_json::from_str(&body_text)
            .map_err(|e| anyhow::anyhow!("Failed to parse embeddings response (status {}): {}. Body: {}", status, e, truncate_str(&body_text, 500)))?;
        resp.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| anyhow::anyhow!("No embedding data in response"))
    }

    /// Send a chat completion request to OpenRouter.
    pub async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> anyhow::Result<ChatCompletionResponse> {
        let response = self
            .http_client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(request)
            .send()
            .await?;

        let status = response.status();
        let body_text = response.text().await.unwrap_or_else(|e| format!("(failed to read body: {})", e));
        if !status.is_success() {
            anyhow::bail!("OpenRouter API error ({}): {}", status, body_text);
        }

        let completion: ChatCompletionResponse = serde_json::from_str(&body_text)
            .map_err(|e| anyhow::anyhow!("Failed to parse chat completion response (status {}): {}. Body: {}", status, e, truncate_str(&body_text, 500)))?;
        Ok(completion)
    }
}
