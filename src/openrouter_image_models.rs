use super::OpenRouterClient;
use serde_json::Value;

impl OpenRouterClient {
    pub async fn list_image_models(&self) -> anyhow::Result<Value> {
        self.image_models_at("https://openrouter.ai/api/v1/images/models", None)
            .await
    }

    pub async fn image_model_details(&self, model: &str) -> anyhow::Result<Value> {
        self.image_models_at("https://openrouter.ai/api/v1/images/models", Some(model))
            .await
    }

    pub(crate) async fn image_models_at(
        &self,
        base: &str,
        model: Option<&str>,
    ) -> anyhow::Result<Value> {
        let mut url = reqwest::Url::parse(base)?;
        if let Some(model) = model {
            let parts: Vec<_> = model.split('/').collect();
            anyhow::ensure!(
                parts.len() == 2
                    && parts
                        .iter()
                        .all(|p| !p.is_empty() && *p != "." && *p != ".."),
                "model must be an author/slug ID"
            );
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid models URL"))?
                .extend(parts)
                .push("endpoints");
        }
        let response = self
            .http_client
            .get(url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        let body = super::openrouter_client::read_response_body(response, "image models").await?;
        let value: Value = body.parse("image models")?;
        if model.is_some() {
            return Ok(value);
        }
        let data = value["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Image models response missing data array"))?;
        let mut models: Vec<Value> = data.iter().map(|item| serde_json::json!({
            "id":item["id"], "name":item["name"], "description":item["description"], "created":item["created"]
        })).collect();
        models.sort_by_key(|item| std::cmp::Reverse(item["created"].as_i64().unwrap_or(0)));
        Ok(serde_json::json!({"data":models}))
    }
}

#[cfg(test)]
#[path = "openrouter_image_models_tests.rs"]
mod tests;
