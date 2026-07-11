use super::*;
use crate::openrouter::{ChatMessage, FunctionDef, ToolDefinition};
use base64::Engine;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn jwt(exp: i64, account: &str) -> String {
    let encode = |value: Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string().as_bytes())
    };
    format!(
        "{}.{}.signature",
        encode(json!({"alg": "none"})),
        encode(json!({
            "exp": exp,
            "https://api.openai.com/auth": {"chatgpt_account_id": account}
        }))
    )
}

fn auth_file(dir: &TempDir, access_token: &str) -> PathBuf {
    let path = dir.path().join("auth.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "tokens": {
                "access_token": access_token,
                "refresh_token": "refresh-token"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn pi_auth_file(dir: &TempDir, access_token: &str) -> PathBuf {
    let path = dir.path().join("pi-auth.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "openai-codex": {
                "type": "oauth",
                "access": access_token,
                "refresh": "refresh-token",
                "expires": 0,
                "accountId": "acct"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn config(server: &MockServer, auth_file: &Path) -> CodexConfig {
    CodexConfig {
        model: "gpt-5.4".into(),
        auth_file: auth_file.display().to_string(),
        reasoning_effort: Some("high".into()),
        base_url: server.uri(),
    }
}

#[tokio::test]
async fn sends_codex_request_and_parses_text_response() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let token = jwt(chrono::Utc::now().timestamp() + 3600, "acct-123");
    let auth = auth_file(&dir, &token);
    let event = json!({
        "type": "response.completed",
        "response": {
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello from Codex"}]
            }],
            "usage": {"input_tokens": 12, "output_tokens": 4, "total_tokens": 16}
        }
    });
    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .and(header("authorization", format!("Bearer {token}")))
        .and(header("chatgpt-account-id", "acct-123"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            "event: response.completed\ndata: {event}\n\ndata: [DONE]\n\n"
        )))
        .mount(&server)
        .await;

    let client = CodexClient::new(config(&server, &auth));
    let response = client
        .chat_completion(&ChatCompletionRequest {
            model: "gpt-5.4".into(),
            messages: vec![ChatMessage::system("Be useful"), ChatMessage::user("Hi")],
            tools: None,
            tool_choice: None,
            modalities: None,
            image_config: None,
        })
        .await
        .unwrap();

    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("Hello from Codex")
    );
    assert_eq!(response.usage.unwrap().total_tokens, 16);
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["instructions"], "Be useful");
    assert_eq!(body["reasoning"]["effort"], "high");
    assert_eq!(body["input"][0]["content"][0]["text"], "Hi");
}

#[tokio::test]
async fn parses_tool_calls_and_replays_provider_state() {
    let response = json!({
        "output": [
            {"type": "reasoning", "id": "rs_1", "encrypted_content": "secret", "summary": [{"type": "summary_text", "text": "Need a tool"}]},
            {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "bash", "arguments": "{\"command\":\"pwd\"}"}
        ],
        "usage": {"input_tokens": 20, "output_tokens": 5}
    });
    let parsed = response_to_chat_completion(&response).unwrap();
    let message = &parsed.choices[0].message;
    assert_eq!(message.reasoning.as_deref(), Some("Need a tool"));
    assert_eq!(message.tool_calls.as_ref().unwrap()[0].id, "call_1");
    assert_eq!(parsed.usage.as_ref().unwrap().total_tokens, 25);

    let continued = ChatMessage::assistant_tool_calls_with_reasoning(
        message.tool_calls.clone().unwrap(),
        message.reasoning.clone().unwrap(),
    )
    .with_provider_data(message.provider_data.clone());
    let body = build_request_body(
        &ChatCompletionRequest {
            model: "gpt-5.4".into(),
            messages: vec![continued, ChatMessage::tool_result("call_1", "/tmp")],
            tools: Some(vec![ToolDefinition {
                def_type: "function".into(),
                function: FunctionDef {
                    name: "bash".into(),
                    description: "Run shell".into(),
                    parameters: json!({"type": "object"}),
                },
            }]),
            tool_choice: None,
            modalities: None,
            image_config: None,
        },
        None,
    )
    .unwrap();
    assert_eq!(body["input"][0]["type"], "reasoning");
    assert_eq!(body["input"][1]["id"], "fc_1");
    assert_eq!(body["input"][2]["type"], "function_call_output");
    assert_eq!(body["tools"][0]["name"], "bash");
}

#[test]
fn replayed_non_codex_tool_calls_use_a_valid_codex_item_id() {
    let call = ToolCall {
        id: "call_function_rkfa35dm1y77_1".into(),
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: "{\"command\":\"pwd\"}".into(),
        },
    };
    let body = build_request_body(
        &ChatCompletionRequest {
            model: "gpt-5.6-terra".into(),
            messages: vec![
                ChatMessage::assistant_tool_calls(vec![call]),
                ChatMessage::tool_result("call_function_rkfa35dm1y77_1", "/tmp"),
            ],
            tools: None,
            tool_choice: None,
            modalities: None,
            image_config: None,
        },
        None,
    )
    .unwrap();
    assert!(body["input"][0]["id"].as_str().unwrap().starts_with("fc_"));
    assert_eq!(body["input"][0]["call_id"], "call_function_rkfa35dm1y77_1");
    assert_eq!(body["input"][1]["call_id"], "call_function_rkfa35dm1y77_1");
}

#[test]
fn request_conversion_supports_names_images_and_rejects_audio() {
    let image = ChatMessage::user_multimodal_with_name(
        vec![
            ContentPart::Text {
                text: "look".into(),
            },
            ContentPart::ImageUrl {
                image_url: crate::openrouter::ImageUrlDetail {
                    url: "data:image/png;base64,AA==".into(),
                    detail: None,
                },
            },
        ],
        "alice",
    );
    let request = ChatCompletionRequest {
        model: "gpt-5.4".into(),
        messages: vec![image],
        tools: None,
        tool_choice: None,
        modalities: None,
        image_config: None,
    };
    let body = build_request_body(&request, None).unwrap();
    assert_eq!(body["input"][0]["content"][0]["text"], "[alice]\nlook");
    assert_eq!(body["input"][0]["content"][1]["type"], "input_image");

    let audio = ChatMessage::user_multimodal(vec![ContentPart::InputAudio {
        input_audio: crate::openrouter::InputAudioDetail {
            data: "AA==".into(),
            format: "wav".into(),
        },
    }]);
    let request = ChatCompletionRequest {
        messages: vec![audio],
        ..request
    };
    assert!(build_request_body(&request, None).is_err());
}

#[test]
fn helpers_handle_sse_urls_jwts_and_models() {
    assert_eq!(
        codex_endpoint("https://example.test/api"),
        "https://example.test/api/codex/responses"
    );
    assert_eq!(
        codex_endpoint("https://example.test/codex"),
        "https://example.test/codex/responses"
    );
    assert_eq!(
        codex_endpoint("https://example.test/codex/responses/"),
        "https://example.test/codex/responses"
    );
    assert_eq!(
        sse_data("data: {\"a\":1}\r\n\r\ndata: [DONE]\r\n\r\n").len(),
        2
    );

    let token = jwt(chrono::Utc::now().timestamp() + 1000, "acct");
    assert_eq!(account_id(&token).unwrap(), "acct");
    assert!(token_valid_for(&token, 10).unwrap());
    assert!(account_id("not-a-jwt").is_err());

    let info = model_info("gpt-5.6-luna");
    assert_eq!(info.context_length, 372_000);
    assert!(info.supports_modality("image"));
    let spark = model_info("gpt-5.3-codex-spark");
    assert_eq!(spark.context_length, 128_000);
    assert!(!spark.supports_modality("image"));

    let unknown = model_info("future-codex-model");
    assert_eq!(unknown.context_length, 0);
    assert!(!unknown.supports_modality("image"));
    assert!(!unknown.supports_modality("audio"));
}

#[tokio::test]
async fn refreshes_expired_credentials_and_persists_them() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let expired = jwt(chrono::Utc::now().timestamp() - 1, "acct");
    let refreshed = jwt(chrono::Utc::now().timestamp() + 3600, "acct");
    let auth = auth_file(&dir, &expired);
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": refreshed,
            "refresh_token": "rotated-refresh",
            "id_token": "new-id-token",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    let token = access_token_with_url(
        &config(&server, &auth),
        &reqwest::Client::new(),
        &Mutex::new(()),
        &format!("{}/oauth/token", server.uri()),
    )
    .await
    .unwrap();
    assert_eq!(token, refreshed);
    let saved: Value = serde_json::from_slice(&std::fs::read(&auth).unwrap()).unwrap();
    assert_eq!(saved["tokens"]["access_token"], refreshed);
    assert_eq!(saved["tokens"]["refresh_token"], "rotated-refresh");
    assert!(saved["last_refresh"].is_string());
}

#[tokio::test]
async fn refreshes_pi_codex_credentials_and_preserves_its_format() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let expired = jwt(chrono::Utc::now().timestamp() - 1, "acct");
    let refreshed_expiry = chrono::Utc::now().timestamp() + 3600;
    let refreshed = jwt(refreshed_expiry, "acct");
    let auth = pi_auth_file(&dir, &expired);
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": refreshed,
            "refresh_token": "rotated-refresh"
        })))
        .mount(&server)
        .await;

    let token = access_token_with_url(
        &config(&server, &auth),
        &reqwest::Client::new(),
        &Mutex::new(()),
        &format!("{}/oauth/token", server.uri()),
    )
    .await
    .unwrap();
    assert_eq!(token, refreshed);
    let saved: Value = serde_json::from_slice(&std::fs::read(&auth).unwrap()).unwrap();
    assert_eq!(saved["openai-codex"]["access"], refreshed);
    assert_eq!(saved["openai-codex"]["refresh"], "rotated-refresh");
    assert_eq!(saved["openai-codex"]["expires"], refreshed_expiry * 1000);
    assert_eq!(saved["openai-codex"]["type"], "oauth");
}

#[tokio::test]
async fn refresh_failure_is_actionable() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let expired = jwt(chrono::Utc::now().timestamp() - 1, "acct");
    let auth = auth_file(&dir, &expired);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid refresh"))
        .mount(&server)
        .await;
    let error = access_token_with_url(
        &config(&server, &auth),
        &reqwest::Client::new(),
        &Mutex::new(()),
        &format!("{}/oauth/token", server.uri()),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("codex login"));
}

#[test]
fn expands_home_and_preserves_absolute_paths() {
    assert_eq!(
        expand_home("/tmp/auth.json").unwrap(),
        PathBuf::from("/tmp/auth.json")
    );
    assert!(expand_home("~/.codex/auth.json")
        .unwrap()
        .ends_with(".codex/auth.json"));
}

#[tokio::test]
async fn missing_auth_file_has_actionable_error() {
    let server = MockServer::start().await;
    let client = CodexClient::new(CodexConfig {
        model: "gpt-5.4".into(),
        auth_file: "/definitely/missing/auth.json".into(),
        reasoning_effort: None,
        base_url: server.uri(),
    });
    let error = client
        .chat_completion(&ChatCompletionRequest {
            model: "gpt-5.4".into(),
            messages: vec![ChatMessage::user("Hi")],
            tools: None,
            tool_choice: None,
            modalities: None,
            image_config: None,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("codex login"));
}

#[tokio::test]
async fn reports_http_and_stream_errors() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let token = jwt(chrono::Utc::now().timestamp() + 3600, "acct");
    let auth = auth_file(&dir, &token);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_string("usage limit"))
        .mount(&server)
        .await;
    let client = CodexClient::new(config(&server, &auth));
    let error = client
        .chat_completion(&ChatCompletionRequest {
            model: "gpt-5.4".into(),
            messages: vec![ChatMessage::user("Hi")],
            tools: None,
            tool_choice: None,
            modalities: None,
            image_config: None,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("429"));

    assert!(parse_sse_response("data: {\"type\":\"response.failed\",\"error\":{}}\n\n").is_err());
    assert!(parse_sse_response("data: [DONE]\n\n").is_err());
}
