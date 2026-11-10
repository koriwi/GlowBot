use super::*;
use serde_json::json;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn catalog_is_sorted_filtered_and_details_are_unfiltered() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
            {"id":"old/model","name":"Old","description":"old","created":10,"pricing":{"cost":1}},
            {"id":"new/model","name":"New","description":"new","created":20,"extra":true}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    let details = json!({"id":"new/model","endpoints":[{"supported_parameters":{"resolution":{"type":"enum","values":["1K"]}},"pricing":[{"cost_usd":0.03}],"unknown_future_field":true}]});
    Mock::given(path("/models/new/model/endpoints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&details))
        .expect(1)
        .mount(&server)
        .await;
    let client = OpenRouterClient::new("key".into());
    let base = format!("{}/models", server.uri());
    let catalog = client.image_models_at(&base, None).await.unwrap();
    assert_eq!(
        catalog,
        json!({"data":[
            {"id":"new/model","name":"New","description":"new","created":20},
            {"id":"old/model","name":"Old","description":"old","created":10}
        ]})
    );
    assert_eq!(
        client
            .image_models_at(&base, Some("new/model"))
            .await
            .unwrap(),
        details
    );
    for model in ["", "no-slash", "../model", "a/b/c"] {
        assert!(client.image_models_at(&base, Some(model)).await.is_err());
    }
}

#[tokio::test]
async fn catalog_errors_are_reported() {
    let server = MockServer::start().await;
    let client = OpenRouterClient::new("key".into());
    for (route, status, body) in [
        ("http", 500, "failure"),
        ("json", 200, "invalid"),
        ("missing", 200, "{}"),
    ] {
        Mock::given(path(format!("/{route}")))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;
        assert!(client
            .image_models_at(&format!("{}/{route}", server.uri()), None)
            .await
            .is_err());
    }
}
