use super::*;
use serde_json::json;
use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[test]
fn dynamic_parameters_are_forwarded_with_harness_boundaries() {
    let mut request =
        ImageGenerationRequest::new("krea/k3-edit".into(), "edit".into(), None, vec![]);
    let parameters = json!({"resolution":"1K", "aspect_ratio":"3:4", "seed":42,
        "future_field":{"anything":[1,true]}, "provider":{"options":{"krea":{"thinking_mode":"fast"}}}});
    request.apply_api_parameters(Some(&parameters)).unwrap();
    let body = serde_json::to_value(&request).unwrap();
    for (key, value) in parameters.as_object().unwrap() {
        assert_eq!(&body[key], value);
    }
    assert_eq!(body["model"], "krea/k3-edit");
    assert!(body.get("api_parameters").is_none());
    assert!(body.get("reference_max_dimension").is_none());
    for invalid in [
        json!([]),
        json!({"model":"other"}),
        json!({"prompt":"other"}),
        json!({"input_references":[]}),
        json!({"reference_max_dimension":100}),
        json!({"reference_images":[]}),
        json!({"api_parameters":{}}),
        json!({"stream":true}),
        json!({"stream":"false"}),
        json!({"size":"1024x1024","resolution":"1K"}),
        json!({"size":"1024x1024","aspect_ratio":"1:1"}),
        json!({"size":"2K","resolution":"1K"}),
    ] {
        assert!(
            request.apply_api_parameters(Some(&invalid)).is_err(),
            "{invalid}"
        );
    }
    let mut legacy =
        ImageGenerationRequest::new("model".into(), "prompt".into(), Some("1K".into()), vec![]);
    assert!(legacy
        .apply_api_parameters(Some(&json!({"resolution":"1K"})))
        .is_err());
    legacy.apply_api_parameters(None).unwrap();
    legacy.apply_api_parameters(Some(&json!(null))).unwrap();
    let mut empty = ImageGenerationRequest::new("model".into(), "prompt".into(), None, vec![]);
    empty
        .apply_api_parameters(Some(&json!({"stream":false})))
        .unwrap();
}

#[tokio::test]
async fn image_api_sends_prompt_dimensions_and_references_and_reads_images() {
    let server = MockServer::start().await;
    let reference = ContentPart::ImageUrl {
        image_url: crate::openrouter::ImageUrlDetail {
            url: "data:image/png;base64,aGVsbG8=".into(),
            detail: None,
        },
    };
    for size in [
        None,
        Some("16:9"),
        Some("1K"),
        Some("2K"),
        Some("4K"),
        Some("512"),
        Some("768"),
        Some("1.5K"),
        Some("2048x2048"),
    ] {
        let mut request = ImageGenerationRequest::new(
            "qwen/qwen-image-2.1-turbo".into(),
            "a cat".into(),
            size.map(str::to_string),
            vec![reference.clone()],
        );
        request
            .apply_api_parameters(Some(&json!({"seed":42,"future_option":true})))
            .unwrap();
        let mut expected = json!({"model":"qwen/qwen-image-2.1-turbo", "prompt":"a cat",
            "seed":42,"future_option":true,
            "input_references":[{"type":"image_url", "image_url":{"url":"data:image/png;base64,aGVsbG8="}}]});
        if let Some(s) = size {
            expected[if s.contains(':') {
                "aspect_ratio"
            } else if matches!(s, "512" | "768" | "1K" | "1.5K" | "2K" | "4K") {
                "resolution"
            } else {
                "size"
            }] = json!(s);
        }
        Mock::given(method("POST"))
            .and(path("/api/v1/images"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_json(expected))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":[{"b64_json":"aGVsbG8=", "media_type":"image/png", "width":1024}, {"b64_json":"d29ybGQ="}],
                "usage":{"cost":0.02,"prompt_tokens":100}, "created":123,
                "future_metadata":{"provider":"example"}
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
        assert_eq!(
            response.metadata(),
            json!({
                "data":[{"media_type":"image/png","width":1024},{}],
                "usage":{"cost":0.02,"prompt_tokens":100},"created":123,
                "future_metadata":{"provider":"example"}
            })
        );
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
