use super::{ContentPart, OpenRouterClient};
use serde::{Deserialize, Serialize};

/// Dedicated Image API request. Reference images use image_url content parts.
#[derive(Debug, Serialize)]
pub struct ImageGenerationRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input_references: Vec<ContentPart>,
}

impl ImageGenerationRequest {
    pub fn new(
        model: String,
        prompt: String,
        size: Option<String>,
        input_references: Vec<ContentPart>,
    ) -> Self {
        let (aspect_ratio, size) = match size {
            Some(s) if s.contains(':') => (Some(s), None),
            other => (None, other),
        };
        Self {
            model,
            prompt,
            aspect_ratio,
            size,
            input_references,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ImageGenerationResponse {
    pub data: Vec<ImageData>,
}

#[derive(Debug, Deserialize)]
pub struct ImageData {
    pub b64_json: String,
    #[serde(default)]
    pub media_type: Option<String>,
}

impl OpenRouterClient {
    pub async fn generate_images(
        &self,
        request: &ImageGenerationRequest,
    ) -> anyhow::Result<ImageGenerationResponse> {
        self.generate_images_at("https://openrouter.ai/api/v1/images", request)
            .await
    }

    pub(crate) async fn generate_images_at(
        &self,
        url: &str,
        request: &ImageGenerationRequest,
    ) -> anyhow::Result<ImageGenerationResponse> {
        let response = self
            .http_client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(request)
            .send()
            .await?;
        let body =
            super::openrouter_client::read_response_body(response, "image generation").await?;
        body.parse("image generation")
    }
}

#[cfg(test)]
#[path = "openrouter_images_tests.rs"]
mod tests;
