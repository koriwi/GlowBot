use super::*;

#[test]
fn probability_threshold_and_validation() {
    for (probability, expected) in [(0.0, false), (0.59, false), (0.6, true), (1.0, true)] {
        let response: DecisionsResponse = serde_json::from_value(
            json!({"answers":{"should_respond":{"type":"noul","noul":probability}}}),
        )
        .unwrap();
        assert_eq!(response.should_respond().unwrap(), expected);
    }
    for value in [
        json!({"answers":{}}),
        json!({"answers":{"should_respond":{"type":"choice","noul":0.8}}}),
        json!({"answers":{"should_respond":{"type":"noul","noul":1.1}}}),
    ] {
        assert!(serde_json::from_value::<DecisionsResponse>(value)
            .unwrap()
            .should_respond()
            .is_err());
    }
}

#[test]
fn state_encodes_images_at_top_level_and_uses_plain_text() {
    let current = ChatMessage::user_multimodal(vec![
        ContentPart::Text {
            text: "What is this?".into(),
        },
        ContentPart::ImageUrl {
            image_url: super::super::ImageUrlDetail {
                url: "data:image/png;base64,YQ==".into(),
                detail: None,
            },
        },
    ]);
    let state = decision_state("glowy", &[ChatMessage::assistant("Hello")], &current);
    assert!(state.iter().any(|part| part == "What is this?"));
    assert!(state.iter().any(|part| part["type"] == "image_url"));
    let request = decision_request("openai/gpt-6-luna-decisions", state);
    assert_eq!(request["questions"]["should_respond"]["type"], "noul");
    assert!(request.get("messages").is_none());
}

#[tokio::test]
async fn decisions_http_contract_and_api_failures() {
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/decisions"))
        .and(header("Authorization", "Bearer test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"answers":{"should_respond":{"type":"noul","noul":0.61}}})),
        )
        .mount(&server)
        .await;
    let client = OpenRouterClient::new("test-key".into());
    assert!(client
        .decide_at(
            &format!("{}/decisions", server.uri()),
            "test/decider",
            vec![json!("hello")],
            "-123"
        )
        .await
        .unwrap());
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "test/decider");
    assert_eq!(body["state"][0], "hello");
    for (status, body) in [
        (429, json!({"error":"rate limited"})),
        (200, json!({"answers":{}})),
        (
            200,
            json!({"answers":{"should_respond":{"type":"noul","noul":-0.1}}}),
        ),
    ] {
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        assert!(client
            .decide_at(
                &format!("{}/decisions", server.uri()),
                "test/decider",
                vec![],
                "-123"
            )
            .await
            .is_err());
    }
}

#[tokio::test]
async fn discovers_decisions_catalog_with_explicit_output_filter() {
    use wiremock::{
        matchers::{method, query_param},
        Mock, MockServer, ResponseTemplate,
    };
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(query_param("output_modalities", "decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"openai/gpt-6-luna-decisions","name":"Luna","context_length":1050000,"architecture":{"input_modalities":["text","image"],"output_modalities":["decisions"]}}]})))
        .expect(1).mount(&server).await;
    let models = OpenRouterClient::new("test-key".into())
        .fetch_decision_models_at(&server.uri())
        .await
        .unwrap();
    assert!(models[0].supports_modality("image"));
    assert!(!models[0].supports_modality("audio"));
}
