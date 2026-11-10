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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input_references: Vec<ContentPart>,
    #[serde(flatten)]
    api_parameters: serde_json::Map<String, serde_json::Value>,
}

impl ImageGenerationRequest {
    pub fn new(
        model: String,
        prompt: String,
        size: Option<String>,
        input_references: Vec<ContentPart>,
    ) -> Self {
        let (aspect_ratio, resolution, size) = match size {
            Some(s) if s.contains(':') => (Some(s), None, None),
            Some(s) if matches!(s.as_str(), "512" | "768" | "1K" | "1.5K" | "2K" | "4K") => {
                (None, Some(s), None)
            }
            other => (None, None, other),
        };
        Self {
            model,
            prompt,
            aspect_ratio,
            size,
            resolution,
            input_references,
            api_parameters: serde_json::Map::new(),
        }
    }

    pub fn apply_api_parameters(
        &mut self,
        value: Option<&serde_json::Value>,
    ) -> anyhow::Result<()> {
        let Some(value) = value.filter(|v| !v.is_null()) else {
            return Ok(());
        };
        let parameters = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("api_parameters must be an object"))?;
        for key in parameters.keys() {
            anyhow::ensure!(
                !matches!(
                    key.as_str(),
                    "model"
                        | "prompt"
                        | "input_references"
                        | "reference_images"
                        | "reference_max_dimension"
                        | "api_parameters"
                ),
                "api_parameters cannot override harness field {key}"
            );
        }
        if let Some(stream) = parameters.get("stream") {
            anyhow::ensure!(
                stream == &serde_json::Value::Bool(false),
                "Only stream: false is supported"
            );
        }
        let legacy = serde_json::to_value(&*self)?;
        for key in ["resolution", "aspect_ratio", "size"] {
            anyhow::ensure!(
                !(parameters.contains_key(key) && legacy.get(key).is_some()),
                "Duplicate {key}: use api_parameters only"
            );
        }
        let size = parameters
            .get("size")
            .or(legacy.get("size"))
            .and_then(|v| v.as_str());
        if size.is_some_and(|s| s.contains('x')) {
            anyhow::ensure!(
                !["resolution", "aspect_ratio"].iter().any(|key| parameters
                    .get(*key)
                    .or(legacy.get(*key))
                    .is_some_and(|v| !v.is_null())),
                "Explicit pixel size cannot be combined with resolution or aspect_ratio"
            );
        }
        let resolution = parameters
            .get("resolution")
            .or(legacy.get("resolution"))
            .and_then(|v| v.as_str());
        if let (Some(size), Some(resolution)) = (size, resolution) {
            anyhow::ensure!(
                size.contains('x') || size == resolution,
                "size tier conflicts with resolution"
            );
        }
        self.api_parameters = parameters.clone();
        Ok(())
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
