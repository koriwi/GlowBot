use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn response_from_wire(wire: &'static [u8], delay: std::time::Duration) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        socket.read(&mut request).await.unwrap();
        socket.write_all(wire).await.unwrap();
        tokio::time::sleep(delay).await;
        socket.shutdown().await.unwrap();
    });
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap()
        .get(format!("http://{address}/response"))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn successful_body_captures_metadata_and_does_not_log_credentials() {
    let response = response_from_wire(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 14\r\nX-Request-Id: req-123\r\nCF-Ray: ray-456\r\nSet-Cookie: secret-session\r\n\r\n{\"choices\":[]}",
        std::time::Duration::ZERO,
    ).await;
    let body = read_response_body(response, "chat completion")
        .await
        .unwrap();
    let completion: ChatCompletionResponse = body.parse("chat completion").unwrap();
    assert!(completion.choices.is_empty());
    for expected in [
        "status 200 OK",
        "application/json",
        "req-123",
        "ray-456",
        "received 14 bytes",
        "body read",
    ] {
        assert!(body.diagnostics.contains(expected), "missing {expected}");
    }
    assert!(!body.diagnostics.contains("secret-session"));
}

#[tokio::test]
async fn incomplete_body_reports_transport_cause_and_partial_body_not_json_error() {
    let response = response_from_wire(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nX-Request-Id: broken-123\r\n\r\n{\"choices\":",
        std::time::Duration::from_millis(20),
    ).await;
    let error = read_response_body(response, "chat completion")
        .await
        .err()
        .unwrap();
    let description = format!("{error:#}");
    for expected in [
        "Failed to read OpenRouter chat completion response body",
        "status 200 OK",
        "broken-123",
        "Partial body",
        "choices",
        "decode=true",
        "timeout=false",
    ] {
        assert!(
            description.contains(expected),
            "missing {expected}: {description}"
        );
    }
    assert!(!description.contains("Failed to parse"));
    assert!(
        error.chain().count() >= 3,
        "transport cause was lost: {description}"
    );
    assert!(error.downcast_ref::<reqwest::Error>().is_some());
}

#[tokio::test]
async fn body_timeout_is_identified() {
    let response = response_from_wire(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n",
        std::time::Duration::from_secs(1),
    )
    .await;
    let error = read_response_body(response, "embeddings")
        .await
        .err()
        .unwrap();
    let description = format!("{error:#}");
    assert!(description.contains("timeout=true"), "{description}");
    assert!(description.contains("received 0 bytes"));
    assert!(error.downcast_ref::<reqwest::Error>().unwrap().is_timeout());
}

#[tokio::test]
async fn invalid_json_has_body_preview_headers_and_parse_cause() {
    let response = response_from_wire(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\n\r\n<html></html>",
        std::time::Duration::ZERO,
    )
    .await;
    let body = read_response_body(response, "chat completion")
        .await
        .unwrap();
    let error = body
        .parse::<ChatCompletionResponse>("chat completion")
        .unwrap_err();
    let description = format!("{error:#}");
    for expected in [
        "Failed to parse chat completion response",
        "text/html",
        "<html></html>",
        "expected value at line 1 column 1",
    ] {
        assert!(
            description.contains(expected),
            "missing {expected}: {description}"
        );
    }
}

#[tokio::test]
async fn api_errors_have_bounded_body_previews() {
    let response = response_from_wire(
        b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 11\r\n\r\nbad gateway",
        std::time::Duration::ZERO,
    )
    .await;
    let mut body = read_response_body(response, "models").await.unwrap();
    body.text = "x".repeat(600);
    let error = body
        .parse::<serde_json::Value>("models")
        .unwrap_err()
        .to_string();
    assert!(error.contains("502 Bad Gateway"));
    assert!(error.contains(&format!("{}...", "x".repeat(500))));
    assert!(!error.contains(&"x".repeat(501)));
}
