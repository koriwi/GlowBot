use teloxide::types::Message;

/// Telegram forwards are genuine user actions, but messages emitted by bots or
/// automatic linked-channel forwarding are not new instructions for GlowBot.
/// This also prevents SMS mirror messages produced by integration bots from
/// entering the LLM pipeline.
pub(super) fn should_ignore_message(message: &Message) -> bool {
    message.from.as_ref().is_some_and(|user| user.is_bot)
        || message.via_bot.is_some()
        || message.is_automatic_forward()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_from_json(extra: serde_json::Value) -> Message {
        let mut root = serde_json::json!({
            "message_id": 1,
            "date": 1_786_643_348,
            "chat": {"id": 42, "type": "private", "first_name": "Kilian"},
            "from": {
                "id": 42,
                "is_bot": false,
                "first_name": "Kilian"
            },
            "text": "hello"
        });
        let root_object = root.as_object_mut().unwrap();
        for (key, extra_value) in extra.as_object().unwrap() {
            root_object.insert(key.clone(), extra_value.clone());
        }
        serde_json::from_value(root).unwrap()
    }

    #[test]
    fn accepts_human_and_manual_forward_messages() {
        assert!(!should_ignore_message(&message_from_json(
            serde_json::json!({})
        )));
        assert!(!should_ignore_message(&message_from_json(
            serde_json::json!({
                "forward_origin": {
                    "type": "hidden_user",
                    "date": 1_786_643_300,
                    "sender_user_name": "Someone"
                }
            })
        )));
    }

    #[test]
    fn ignores_bot_via_bot_and_automatic_forward_messages() {
        assert!(should_ignore_message(&message_from_json(
            serde_json::json!({
                "from": {"id": 99, "is_bot": true, "first_name": "SMS Mirror Bot"}
            })
        )));
        assert!(should_ignore_message(&message_from_json(
            serde_json::json!({
                "via_bot": {"id": 99, "is_bot": true, "first_name": "Integration Bot"}
            })
        )));
        assert!(should_ignore_message(&message_from_json(
            serde_json::json!({
                "is_automatic_forward": true
            })
        )));
    }
}
