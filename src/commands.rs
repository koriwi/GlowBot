use crate::config::{ChatConfig, Config};

/// Result of parsing a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// /status
    Status,
    /// /stop
    Stop,
    /// /tasks
    Tasks,
}

/// Parse a Telegram message to see if it's a bot command.
pub fn parse_command(text: &str) -> Option<Command> {
    let text = text.trim();
    if !text.starts_with('/') {
        return None;
    }

    let (cmd, _args) = match text.split_once(' ') {
        Some((c, a)) => (c.trim(), a.trim()),
        None => (text, ""),
    };

    match cmd {
        "/status" => Some(Command::Status),
        "/stop" => Some(Command::Stop),
        "/tasks" => Some(Command::Tasks),
        _ => None,
    }
}

/// Check if a user is allowed to run commands in a chat.
pub fn can_run_command(chat_config: &ChatConfig, user_id: &str) -> bool {
    if chat_config.command_whitelist.is_empty() {
        return false; // Empty = nobody
    }
    chat_config.command_whitelist.contains(&user_id.to_string())
}

/// Check if a user is allowed to interact with the bot in a chat.
pub fn can_interact(chat_config: &ChatConfig, user_id: &str) -> bool {
    if chat_config.interaction_whitelist.is_empty() {
        return true; // Empty = everyone
    }
    chat_config
        .interaction_whitelist
        .contains(&user_id.to_string())
}

/// Handle a command and return the response text, optionally mutating config.
pub fn handle_command(
    command: &Command,
    config: &mut Config,
    chat_id: &str,
    context_usage: &str,
) -> String {
    match command {
        Command::Status => {
            let chat = config.chat_config(chat_id);
            let model = chat
                .model
                .as_deref()
                .unwrap_or(&config.openrouter_default_model);
            format!(
                "Chat ID: {}\nModel: {}\nContext usage: {}\nInteraction mode: {:?}\nInteraction whitelist: {}\nCommand whitelist: {}",
                chat_id,
                model,
                context_usage,
                chat.interaction_mode,
                if chat.interaction_whitelist.is_empty() {
                    "everyone".to_string()
                } else {
                    chat.interaction_whitelist.join(", ")
                },
                if chat.command_whitelist.is_empty() {
                    "nobody".to_string()
                } else {
                    chat.command_whitelist.join(", ")
                },
            )
        }
        Command::Stop => "Stop command received.".to_string(),
        Command::Tasks => String::new(), // handled in handle_bot_command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_status() {
        assert_eq!(parse_command("/status"), Some(Command::Status));
    }

    #[test]
    fn test_parse_command_stop() {
        assert_eq!(parse_command("/stop"), Some(Command::Stop));
    }

    #[test]
    fn test_parse_not_a_command() {
        assert!(parse_command("Hello!").is_none());
        assert!(parse_command("").is_none());
        assert!(parse_command("   hi   ").is_none());
        // /model, /mode, /reload are no longer commands
        assert!(parse_command("/model gpt-4").is_none());
        assert!(parse_command("/mode every_message").is_none());
        assert!(parse_command("/reload").is_none());
    }

    #[test]
    fn test_can_run_command() {
        let config = ChatConfig {
            command_whitelist: vec!["123".into()],
            ..Default::default()
        };
        assert!(can_run_command(&config, "123"));
        assert!(!can_run_command(&config, "456"));
        // Empty whitelist = nobody
        let config = ChatConfig::default();
        assert!(!can_run_command(&config, "123"));
    }

    #[test]
    fn test_can_interact() {
        // Empty whitelist = everyone
        let config = ChatConfig::default();
        assert!(can_interact(&config, "anyone"));
        // With whitelist
        let config = ChatConfig {
            interaction_whitelist: vec!["123".into()],
            ..Default::default()
        };
        assert!(can_interact(&config, "123"));
        assert!(!can_interact(&config, "456"));
    }

    #[test]
    fn test_handle_status_command() {
        let mut config = crate::config::basic_config();
        config.openrouter_default_model = "default-model".into();
        let resp = handle_command(&Command::Status, &mut config, "-123", "1k/10k (10%)");
        assert!(resp.contains("-123"));
        assert!(resp.contains("default-model"));
        assert!(resp.contains("1k/10k (10%)"));
        assert!(resp.contains("MentionOnly"));
        assert!(resp.contains("everyone"));
        assert!(resp.contains("nobody"));
    }

    #[test]
    fn test_handle_stop_command() {
        let mut config = crate::config::basic_config();
        let resp = handle_command(&Command::Stop, &mut config, "-123", "");
        assert!(resp.contains("Stop command received"));
    }
}
