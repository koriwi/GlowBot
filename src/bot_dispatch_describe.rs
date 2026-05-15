use super::BotState;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle the `describe_image` tool — call the image fallback model with a custom prompt.
pub(crate) async fn tool_describe_image(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    args: &serde_json::Value,
) -> String {
    let file_path = args["file_path"].as_str().unwrap_or("");
    let prompt = args["prompt"].as_str().unwrap_or("");

    if file_path.is_empty() || prompt.is_empty() {
        return "Error: file_path and prompt required".into();
    }

    let (fallback_model, api_key) = {
        let s = state.lock().await;
        let model = match s.config.image_fallback_model_for_chat(chat_id) {
            Some(m) => m.to_string(),
            None => {
                return "Error: no image fallback model configured — describe_image is disabled."
                    .into()
            }
        };
        let key = s.config.openrouter.api_key.clone();
        (model, key)
    };

    let path = std::path::Path::new(file_path);

    let data_url = match crate::media::image_to_data_url(path) {
        Ok(d) => d,
        Err(e) => return format!("Error reading image file '{}': {}", file_path, e),
    };

    let parts = vec![
        crate::openrouter::ContentPart::Text {
            text: prompt.to_string(),
        },
        crate::openrouter::ContentPart::ImageUrl {
            image_url: crate::openrouter::ImageUrlDetail {
                url: data_url,
                detail: None,
            },
        },
    ];

    let msg = crate::openrouter::ChatMessage::user_multimodal(parts);
    let request = crate::openrouter::ChatCompletionRequest {
        model: fallback_model,
        messages: vec![msg],
        tools: None,
        tool_choice: None,
        modalities: None,
        image_config: None,
    };

    let client = crate::openrouter::OpenRouterClient::new(api_key);

    match client.chat_completion(&request).await {
        Ok(resp) => {
            let text = resp
                .choices
                .into_iter()
                .next()
                .and_then(|c| c.message.content)
                .unwrap_or_default();
            if text.is_empty() {
                "The image model returned an empty response.".into()
            } else {
                text
            }
        }
        Err(e) => format!("Error calling image fallback model: {}", e),
    }
}
