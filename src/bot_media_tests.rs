use super::*;

#[test]
fn decider_uses_native_images_but_transcribes_audio() {
    let mut model = crate::codex::model_info("test");
    model.architecture.input_modalities = vec!["text".into(), "image".into(), "audio".into()];
    assert!(native_supported(Some(&model), "image", true));
    assert!(!native_supported(Some(&model), "audio", true));
    assert!(native_supported(Some(&model), "audio", false));
    model.architecture.input_modalities = vec!["text".into()];
    assert!(!native_supported(Some(&model), "image", true));
    assert!(!native_supported(None, "image", true));
}

#[test]
fn native_image_preserves_file_and_caption_for_agent_and_judge() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("photo.png");
    std::fs::write(&path, b"image bytes").unwrap();
    let media = crate::media::IngestedMedia::Photo {
        file_id: "photo".into(),
        width: 10,
        height: 10,
    };
    let message = build_native_message(&media, Some("What is this?"), "", "alice", "Alice", &path);
    let payload = crate::openrouter::decision_state("mybot", &[], &message);
    assert!(payload.iter().any(|p| p["type"] == "image_url"));
    assert!(payload.iter().any(|p| p == "What is this?"));
    let stripped = crate::bot::bot_decider::without_images(message, false);
    assert!(stripped.text_content().contains(path.to_str().unwrap()));
    assert!(stripped.text_content().contains("What is this?"));
}
