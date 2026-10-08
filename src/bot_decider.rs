use super::BotState;
use crate::openrouter::{
    ChatCompletionRequest, ChatContent, ChatMessage, ContentPart, OpenRouterClient,
};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) async fn refresh(state: &Arc<Mutex<BotState>>) -> anyhow::Result<()> {
    let key = state.lock().await.config.openrouter.api_key.clone();
    let client = OpenRouterClient::new(key);
    #[cfg(test)]
    let models = if let Some(url) = state.lock().await.decision_test_url.clone() {
        client.fetch_decision_models_at(&url).await?
    } else {
        client.fetch_decision_models().await?
    };
    #[cfg(not(test))]
    let models = client.fetch_decision_models().await?;
    let mut state = state.lock().await;
    for model in models {
        state.decision_metadata.insert(model.id.clone(), model);
    }
    Ok(())
}

pub(crate) async fn model_for_decision(state: &Arc<Mutex<BotState>>) -> anyhow::Result<String> {
    let model = state
        .lock()
        .await
        .config
        .openrouter
        .decider_model
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No decider model configured"))?;
    if !state.lock().await.decision_metadata.contains_key(&model) {
        tokio::time::timeout(std::time::Duration::from_secs(15), refresh(state)).await??;
    }
    anyhow::ensure!(
        state.lock().await.decision_metadata.contains_key(&model),
        "Decider model not found in Decisions catalog: {model}"
    );
    Ok(model)
}

pub(crate) async fn judge(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    model: &str,
    bot_username: &str,
    history: &[ChatMessage],
    current: &ChatMessage,
) -> anyhow::Result<bool> {
    let (key, supports_image) = {
        let s = state.lock().await;
        (
            s.config.openrouter.api_key.clone(),
            s.decision_metadata
                .get(model)
                .is_some_and(|m| m.supports_modality("image")),
        )
    };
    // Historical media is represented by its saved text/path for text-only deciders.
    let history: Vec<_> = history
        .iter()
        .cloned()
        .map(|m| without_images(m, supports_image))
        .collect();
    let payload = crate::openrouter::decision_state(bot_username, &history, current);
    #[cfg(test)]
    if let Some(url) = state.lock().await.decision_test_url.clone() {
        return OpenRouterClient::new(key)
            .decide_at(&url, model, payload, chat_id)
            .await;
    }
    OpenRouterClient::new(key)
        .decide(model, payload, chat_id)
        .await
}

pub(crate) fn without_images(mut message: ChatMessage, supports_image: bool) -> ChatMessage {
    if !supports_image {
        if let ChatContent::Parts(ref mut parts) = message.content {
            parts.retain(|part| !matches!(part, ContentPart::ImageUrl { .. }));
        }
    }
    message
}

pub(crate) async fn for_agent(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    message: ChatMessage,
) -> ChatMessage {
    let s = state.lock().await;
    let model = s.effective_model(chat_id);
    let supports_image = s
        .model_metadata
        .get(crate::openrouter::normalize_model_id(&model))
        .is_some_and(|m| m.supports_modality("image"));
    without_images(message, supports_image)
}

pub(crate) async fn describe(
    key: &str,
    model: &str,
    path: &std::path::Path,
) -> anyhow::Result<String> {
    let request = ChatCompletionRequest {
        model: model.into(),
        messages: vec![ChatMessage::user_multimodal(vec![
            ContentPart::Text { text: "Describe this image, including visible text, so another model can understand the user's message.".into() },
            ContentPart::ImageUrl { image_url: crate::openrouter::ImageUrlDetail { url: crate::media::image_to_data_url(path)?, detail: None } },
        ])],
        tools: None, tool_choice: None, modalities: None, image_config: None,
    };
    let response = OpenRouterClient::new(key.into())
        .chat_completion(&request)
        .await?;
    let text = response
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Empty image description"))?;
    Ok(text)
}
