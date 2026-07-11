use crate::config::CodexConfig;
use crate::openrouter::{
    AssistantMessage, ChatCompletionRequest, ChatCompletionResponse, ChatContent, Choice,
    ContentPart, FunctionCall, ToolCall, Usage,
};
use anyhow::Context;
use serde_json::{json, Value};
use tokio::sync::Mutex;

#[path = "codex_auth.rs"]
mod codex_auth;
use codex_auth::{access_token, account_id};
#[cfg(test)]
use codex_auth::{access_token_with_url, expand_home, token_valid_for};

/// Client for OpenAI's Codex Responses endpoint, authenticated with credentials
/// produced by the official `codex login` command.
pub struct CodexClient {
    config: CodexConfig,
    http_client: reqwest::Client,
    auth_lock: Mutex<()>,
}

impl CodexClient {
    pub fn new(config: CodexConfig) -> Self {
        Self {
            config,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .connect_timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build Codex HTTP client"),
            auth_lock: Mutex::new(()),
        }
    }

    pub async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> anyhow::Result<ChatCompletionResponse> {
        let token = access_token(&self.config, &self.http_client, &self.auth_lock).await?;
        let account_id = account_id(&token)?;
        let body = build_request_body(request, self.config.reasoning_effort.as_deref())?;
        let endpoint = codex_endpoint(&self.config.base_url);

        let response = self
            .http_client
            .post(&endpoint)
            .bearer_auth(&token)
            .header("chatgpt-account-id", account_id)
            .header("originator", "glowbot")
            .header("User-Agent", "glowbot")
            .header("OpenAI-Beta", "responses=experimental")
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|e| format!("(failed to read body: {e})"));
        if !status.is_success() {
            anyhow::bail!("Codex API error ({status}): {}", truncate(&body_text, 1000));
        }
        parse_sse_response(&body_text)
    }
}

fn build_request_body(
    request: &ChatCompletionRequest,
    reasoning_effort: Option<&str>,
) -> anyhow::Result<Value> {
    let mut instructions = Vec::new();
    let mut input = Vec::new();

    for message in &request.messages {
        if message.role == "system" {
            instructions.push(message.text_content());
            continue;
        }
        if message.role == "tool" {
            input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id,
                "output": message.text_content(),
            }));
            continue;
        }

        if message.role == "assistant" {
            let mut has_provider_calls = false;
            if let Some(items) = message.provider_data.as_ref().and_then(Value::as_array) {
                for item in items {
                    if matches!(
                        item.get("type").and_then(Value::as_str),
                        Some("reasoning" | "function_call")
                    ) {
                        has_provider_calls |=
                            item.get("type").and_then(Value::as_str) == Some("function_call");
                        input.push(item.clone());
                    }
                }
            }
            let text = message.text_content();
            if !text.is_empty() {
                input.push(json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text}],
                }));
            }
            if !has_provider_calls {
                append_function_calls(&mut input, message.tool_calls.as_deref());
            }
            continue;
        }

        let mut content = Vec::new();
        match &message.content {
            ChatContent::Text(text) => {
                let text = with_name(text, message.name.as_deref());
                content.push(json!({"type": "input_text", "text": text}));
            }
            ChatContent::Parts(parts) => {
                for part in parts {
                    match part {
                        ContentPart::Text { text } => content.push(json!({
                            "type": "input_text",
                            "text": with_name(text, message.name.as_deref()),
                        })),
                        ContentPart::ImageUrl { image_url } => content.push(json!({
                            "type": "input_image",
                            "image_url": image_url.url,
                            "detail": image_url.detail.as_deref().unwrap_or("auto"),
                        })),
                        ContentPart::InputAudio { .. } => anyhow::bail!(
                            "Codex subscription models do not support GlowBot's input_audio format"
                        ),
                    }
                }
            }
        }
        input.push(json!({"type": "message", "role": "user", "content": content}));
    }

    let tools: Vec<Value> = request
        .tools
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.function.name,
                "description": tool.function.description,
                "parameters": tool.function.parameters,
                "strict": false,
            })
        })
        .collect();

    let mut body = json!({
        "model": request.model,
        "store": false,
        "stream": true,
        "instructions": if instructions.is_empty() { "You are a helpful assistant.".into() } else { instructions.join("\n\n") },
        "input": input,
        "tools": tools,
        "tool_choice": request.tool_choice.as_deref().unwrap_or("auto"),
        "parallel_tool_calls": true,
        "include": ["reasoning.encrypted_content"],
        "text": {"verbosity": "medium"},
    });
    if let Some(effort) = reasoning_effort {
        body["reasoning"] = json!({"effort": effort, "summary": "auto"});
    }
    Ok(body)
}

fn append_function_calls(input: &mut Vec<Value>, calls: Option<&[ToolCall]>) {
    for call in calls.unwrap_or_default() {
        // Historic calls may originate from OpenRouter, whose call IDs (for
        // example `call_function_…`) are not valid Responses `id` values.
        // The Codex endpoint requires this internal item ID to begin with
        // `fc`; the user-visible call_id still links it to its tool result.
        input.push(json!({
            "type": "function_call",
            "id": format!("fc_{}", uuid::Uuid::new_v4().simple()),
            "call_id": call.id,
            "name": call.function.name,
            "arguments": call.function.arguments,
        }));
    }
}

fn with_name(text: &str, name: Option<&str>) -> String {
    name.map(|name| format!("[{name}]\n{text}"))
        .unwrap_or_else(|| text.to_string())
}

fn parse_sse_response(body: &str) -> anyhow::Result<ChatCompletionResponse> {
    let mut completed = None;
    let mut api_error = None;
    for data in sse_data(body) {
        if data == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(&data)
            .with_context(|| format!("Invalid JSON in Codex event: {}", truncate(&data, 300)))?;
        match event.get("type").and_then(Value::as_str) {
            Some("response.completed" | "response.incomplete") => {
                completed = event.get("response").cloned();
            }
            Some("response.failed" | "error") => api_error = Some(event),
            _ => {}
        }
    }
    if let Some(error) = api_error {
        anyhow::bail!(
            "Codex response failed: {}",
            truncate(&error.to_string(), 1000)
        );
    }
    let response = completed.context("Codex stream ended without a completed response")?;
    response_to_chat_completion(&response)
}

fn response_to_chat_completion(response: &Value) -> anyhow::Result<ChatCompletionResponse> {
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut text = String::new();
    let mut reasoning = Vec::new();
    let mut tool_calls = Vec::new();
    let mut provider_items = Vec::new();

    for item in &output {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        if matches!(
                            part.get("type").and_then(Value::as_str),
                            Some("output_text")
                        ) {
                            if let Some(value) = part.get("text").and_then(Value::as_str) {
                                text.push_str(value);
                            }
                        }
                    }
                }
            }
            Some("reasoning") => {
                if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                    reasoning.extend(summary.iter().filter_map(|part| {
                        part.get("text").and_then(Value::as_str).map(str::to_string)
                    }));
                }
                provider_items.push(item.clone());
            }
            Some("function_call") => {
                let id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .context("Codex function call has no call_id")?;
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .context("Codex function call has no name")?;
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                tool_calls.push(ToolCall {
                    id: id.into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: name.into(),
                        arguments: arguments.into(),
                    },
                });
                provider_items.push(item.clone());
            }
            _ => {}
        }
    }

    let usage_value = response.get("usage").unwrap_or(&Value::Null);
    let prompt_tokens = usage_value
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage_value
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage_value
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(prompt_tokens + completion_tokens);
    let has_tools = !tool_calls.is_empty();

    Ok(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some(text),
                tool_calls: has_tools.then_some(tool_calls),
                reasoning: (!reasoning.is_empty()).then(|| reasoning.join("\n")),
                provider_data: (!provider_items.is_empty()).then(|| Value::Array(provider_items)),
                role: Some("assistant".into()),
                images: None,
            },
            finish_reason: Some(if has_tools { "tool_calls" } else { "stop" }.into()),
        }],
        usage: Some(Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        }),
    })
}

fn sse_data(body: &str) -> Vec<String> {
    body.replace("\r\n", "\n")
        .split("\n\n")
        .filter_map(|block| {
            let lines: Vec<&str> = block
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect();
            (!lines.is_empty()).then(|| lines.join("\n"))
        })
        .collect()
}

fn codex_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/codex/responses") {
        base.into()
    } else if base.ends_with("/codex") {
        format!("{base}/responses")
    } else {
        format!("{base}/codex/responses")
    }
}

/// Codex models offered by the interactive model picker. Users can still set
/// another subscription-entitled Codex model with `/model <model-id>`.
pub const KNOWN_MODELS: &[(&str, &str)] = &[
    ("gpt-5.3-codex-spark", "GPT-5.3 Codex Spark"),
    ("gpt-5.4", "GPT-5.4"),
    ("gpt-5.4-mini", "GPT-5.4 mini"),
    ("gpt-5.5", "GPT-5.5"),
    ("gpt-5.6-luna", "GPT-5.6 Luna"),
    ("gpt-5.6-sol", "GPT-5.6 Sol"),
    ("gpt-5.6-terra", "GPT-5.6 Terra"),
];

/// Build metadata for a configured Codex model without depending on OpenRouter's catalog.
pub fn model_info(model: &str) -> crate::openrouter::ModelInfo {
    let (context_length, input_modalities) = match model {
        "gpt-5.3-codex-spark" => (128_000, vec!["text".into()]),
        "gpt-5.4" | "gpt-5.4-mini" | "gpt-5.5" => (272_000, vec!["text".into(), "image".into()]),
        "gpt-5.6-luna" | "gpt-5.6-sol" | "gpt-5.6-terra" => {
            (372_000, vec!["text".into(), "image".into()])
        }
        // Codex's subscription endpoint has no model-metadata API. Avoid
        // sending unsupported media when a user enters an unlisted model ID.
        _ => (0, vec!["text".into()]),
    };
    crate::openrouter::ModelInfo {
        id: model.into(),
        name: KNOWN_MODELS
            .iter()
            .find(|(id, _)| *id == model)
            .map(|(_, name)| format!("OpenAI Codex: {name}"))
            .unwrap_or_else(|| format!("OpenAI Codex: {model}")),
        created: 0,
        context_length,
        architecture: crate::openrouter::ModelArchitecture { input_modalities },
        pricing: crate::openrouter::ModelPricing::default(),
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.into()
    } else {
        format!("{}...", value.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
