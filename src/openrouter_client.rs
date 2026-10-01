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
}

impl OpenRouterClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build reqwest client"),
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

        let body = read_response_body(response, "models").await?;

        #[derive(serde::Deserialize)]
        struct ModelsApiResponse {
            data: Vec<ModelInfo>,
        }

        let resp: ModelsApiResponse = body.parse("models")?;
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

        let body = read_response_body(response, "embeddings").await?;
        let resp: EmbeddingResponse = body.parse("embeddings")?;
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

        let body = read_response_body(response, "chat completion").await?;
        let completion: ChatCompletionResponse = body.parse("chat completion")?;
        if completion.choices.is_empty() {
            log::warn!(
                "OpenRouter returned 200 OK but with empty choices (likely a provider error wrapped by OpenRouter). Body: {}",
                truncate_str(&body.text, 500)
            );
        }
        Ok(completion)
    }
}

/// Keep response metadata and body failures separate from JSON decoding failures.
struct ResponseBody {
    status: reqwest::StatusCode,
    text: String,
    diagnostics: String,
}

impl ResponseBody {
    fn parse<T: serde::de::DeserializeOwned>(&self, operation: &str) -> anyhow::Result<T> {
        use anyhow::Context;
        if !self.status.is_success() {
            anyhow::bail!(
                "OpenRouter {operation} API error: {}. Body: {:?}",
                self.diagnostics,
                truncate_str(&self.text, 500)
            );
        }
        serde_json::from_str(&self.text).with_context(|| {
            format!(
                "Failed to parse {operation} response: {}. Body: {:?}",
                self.diagnostics,
                truncate_str(&self.text, 500)
            )
        })
    }
}

async fn read_response_body(
    mut response: reqwest::Response,
    operation: &str,
) -> anyhow::Result<ResponseBody> {
    let status = response.status();
    let mut diagnostics = format!("status {status}, HTTP {:?}", response.version());
    // Only response headers useful for transport/provider diagnostics. Never log request credentials.
    for name in [
        "content-type",
        "content-length",
        "content-encoding",
        "transfer-encoding",
        "x-request-id",
        "x-openrouter-request-id",
        "cf-ray",
        "server",
    ] {
        if let Some(value) = response.headers().get(name) {
            diagnostics.push_str(&format!(", {name}={value:?}"));
        }
    }
    let started = std::time::Instant::now();
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
            Ok(None) => break,
            Err(error) => {
                let timeout = error.is_timeout();
                let decode = error.is_decode();
                let body_error = error.is_body();
                let error = anyhow::Error::new(error).context(format!(
                    "Failed to read OpenRouter {operation} response body: {diagnostics}, received {} bytes, body read {} ms, timeout={timeout}, decode={decode}, body_error={body_error}. Partial body: {:?}",
                    bytes.len(), started.elapsed().as_millis(), truncate_str(&String::from_utf8_lossy(&bytes), 500)));
                log::error!("{error:#}");
                return Err(error);
            }
        }
    }
    diagnostics.push_str(&format!(
        ", received {} bytes, body read {} ms",
        bytes.len(),
        started.elapsed().as_millis()
    ));
    Ok(ResponseBody {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        diagnostics,
    })
}

#[cfg(test)]
#[path = "openrouter_client_tests.rs"]
mod tests;
