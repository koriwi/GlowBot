use super::*;
use serde_json::json;
use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn image_api_sends_prompt_dimensions_and_references_and_reads_images() {
    let server = MockServer::start().await;
    let reference = ContentPart::ImageUrl {
        image_url: crate::openrouter::ImageUrlDetail {
            url: "data:image/png;base64,aGVsbG8=".into(),
            detail: None,
        },
    };
    for size in [None, Some("16:9"), Some("2K"), Some("2048x2048")] {
        let request = ImageGenerationRequest::new(
            "qwen/qwen-image-2.1-turbo".into(),
            "a cat".into(),
            size.map(str::to_string),
            vec![reference.clone()],
        );
        let mut expected = json!({"model":"qwen/qwen-image-2.1-turbo", "prompt":"a cat",
            "input_references":[{"type":"image_url", "image_url":{"url":"data:image/png;base64,aGVsbG8="}}]});
        if let Some(s) = size {
            expected[if s.contains(':') {
                "aspect_ratio"
            } else {
                "size"
            }] = json!(s);
        }
        Mock::given(method("POST"))
            .and(path("/api/v1/images"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_json(expected))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":[{"b64_json":"aGVsbG8=", "media_type":"image/png"}, {"b64_json":"d29ybGQ="}],
                "usage":{"cost":0.02}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let response = OpenRouterClient::new("test-key".into())
            .generate_images_at(&format!("{}/api/v1/images", server.uri()), &request)
            .await
            .unwrap();
        assert_eq!(response.data.len(), 2);
        assert_eq!(response.data[0].b64_json, "aGVsbG8=");
        assert_eq!(response.data[0].media_type.as_deref(), Some("image/png"));
        assert!(response.data[1].media_type.is_none());
    }
}

#[tokio::test]
async fn image_api_handles_empty_results_and_errors() {
    let server = MockServer::start().await;
    let client = OpenRouterClient::new("test-key".into());
    let request = ImageGenerationRequest::new("model".into(), "prompt".into(), None, vec![]);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"model":"model", "prompt":"prompt"})
    );
    for (route, status, body, error) in [
        ("empty", 200, r#"{"data":[]}"#, None),
        (
            "failure",
            400,
            r#"{"error":{"message":"unsupported size"}}"#,
            Some("400 Bad Request"),
        ),
        (
            "invalid",
            200,
            "not json",
            Some("Failed to parse image generation response"),
        ),
        (
            "missing",
            200,
            "{}",
            Some("Failed to parse image generation response"),
        ),
    ] {
        Mock::given(path(format!("/{route}")))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let result = client
            .generate_images_at(&format!("{}/{route}", server.uri()), &request)
            .await;
        if let Some(message) = error {
            assert!(result.unwrap_err().to_string().contains(message));
        } else {
            assert!(result.unwrap().data.is_empty());
        }
    }
}
