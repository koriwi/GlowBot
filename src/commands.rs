use crate::config::{ChatConfig, Config, InteractionMode};

/// Result of parsing a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// /model <model_name>
    Model(String),
    /// /mode every_message|mention_only
    Mode(String),
    /// /reload
    Reload,
    /// /status
    Status,
}

/// Parse a Telegram message to see if it's a bot command.
pub fn parse_command(text: &str) -> Option<Command> {
    let text = text.trim();
    if !text.starts_with('/') {
        return None;
    }

    let (cmd, args) = match text.split_once(' ') {
        Some((c, a)) => (c.trim(), a.trim()),
        None => (text, ""),
    };

    match cmd {
        "/model" => {
            if args.is_empty() {
                None
            } else {
                Some(Command::Model(args.to_string()))
            }
        }
        "/mode" => {
            if args.is_empty() {
                None
            } else {
                Some(Command::Mode(args.to_string()))
            }
        }
        "/reload" => Some(Command::Reload),
        "/status" => Some(Command::Status),
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
pub fn handle_command(command: &Command, config: &mut Config, chat_id: &str) -> String {
    match command {
        Command::Model(model_name) => {
            let chat = config.chats.entry(chat_id.to_string()).or_default();
            chat.model = Some(model_name.clone());
            format!("Model for this chat set to: {}", model_name)
        }
        Command::Mode(mode_str) => {
            let mode = match mode_str.as_str() {
                "every_message" | "every" => InteractionMode::EveryMessage,
                "mention_only" | "mention" => InteractionMode::MentionOnly,
                other => {
                    return format!(
                        "Unknown mode: {}. Use 'every_message' or 'mention_only'.",
                        other
                    );
                }
            };
            let chat = config.chats.entry(chat_id.to_string()).or_default();
            chat.interaction_mode = mode.clone();
            format!("Interaction mode for this chat set to: {:?}", mode)
        }
        Command::Reload => "Skills reloaded successfully.".to_string(),
        Command::Status => {
            let chat = config.chat_config(chat_id);
            let model = chat
                .model
                .as_deref()
                .unwrap_or(&config.openrouter_default_model);
            format!(
                "Chat ID: {}\nModel: {}\nInteraction mode: {:?}\nInteraction whitelist: {}\nCommand whitelist: {}",
                chat_id,
                model,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_model() {
        assert_eq!(
            parse_command("/model gpt-4"),
            Some(Command::Model("gpt-4".into()))
        );
        assert_eq!(
            parse_command("/model anthropic/claude-sonnet-4"),
            Some(Command::Model("anthropic/claude-sonnet-4".into()))
        );
        assert!(parse_command("/model").is_none()); // needs arg
    }

    #[test]
    fn test_parse_command_mode() {
        assert_eq!(
            parse_command("/mode every_message"),
            Some(Command::Mode("every_message".into()))
        );
        assert_eq!(
            parse_command("/mode mention_only"),
            Some(Command::Mode("mention_only".into()))
        );
        assert!(parse_command("/mode").is_none()); // needs arg
    }

    #[test]
    fn test_parse_command_reload() {
        assert_eq!(parse_command("/reload"), Some(Command::Reload));
    }

    #[test]
    fn test_parse_command_status() {
        assert_eq!(parse_command("/status"), Some(Command::Status));
    }

    #[test]
    fn test_parse_not_a_command() {
        assert!(parse_command("Hello!").is_none());
        assert!(parse_command("").is_none());
        assert!(parse_command("   hi   ").is_none());
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
    fn test_handle_model_command() {
        let mut config = Config {
            telegram_token: "t".into(),
            openrouter_api_key: "k".into(),
            openrouter_default_model: "default".into(),
            chats: std::collections::HashMap::new(),
        };
        let resp = handle_command(&Command::Model("custom/model".into()), &mut config, "-123");
        assert!(resp.contains("custom/model"));
        assert_eq!(config.chat_config("-123").model.unwrap(), "custom/model");
    }

    #[test]
    fn test_handle_mode_command() {
        let mut config = Config {
            telegram_token: "t".into(),
            openrouter_api_key: "k".into(),
            openrouter_default_model: "d".into(),
            chats: std::collections::HashMap::new(),
        };
        let resp = handle_command(&Command::Mode("every_message".into()), &mut config, "-123");
        assert!(resp.contains("EveryMessage"));
        assert_eq!(
            config.chat_config("-123").interaction_mode,
            InteractionMode::EveryMessage
        );

        let resp = handle_command(&Command::Mode("mention_only".into()), &mut config, "-123");
        assert!(resp.contains("MentionOnly"));

        let resp = handle_command(&Command::Mode("invalid".into()), &mut config, "-123");
        assert!(resp.contains("Unknown mode"));
    }

    #[test]
    fn test_handle_reload_command() {
        let mut config = Config {
            telegram_token: "t".into(),
            openrouter_api_key: "k".into(),
            openrouter_default_model: "d".into(),
            chats: std::collections::HashMap::new(),
        };
        let resp = handle_command(&Command::Reload, &mut config, "-123");
        assert_eq!(resp, "Skills reloaded successfully.");
    }

    #[test]
    fn test_handle_status_command() {
        let config = Config {
            telegram_token: "t".into(),
            openrouter_api_key: "k".into(),
            openrouter_default_model: "default-model".into(),
            chats: std::collections::HashMap::new(),
        };
        let resp = handle_command(&Command::Status, &mut config.clone(), "-123");
        assert!(resp.contains("-123"));
        assert!(resp.contains("default-model"));
        assert!(resp.contains("MentionOnly"));
        assert!(resp.contains("everyone"));
        assert!(resp.contains("nobody"));
    }

    #[test]
    fn test_handle_mode_shorthand() {
        let mut config = Config {
            telegram_token: "t".into(),
            openrouter_api_key: "k".into(),
            openrouter_default_model: "d".into(),
            chats: std::collections::HashMap::new(),
        };
        let resp = handle_command(&Command::Mode("every".into()), &mut config, "-123");
        assert!(resp.contains("EveryMessage"));

        let resp = handle_command(&Command::Mode("mention".into()), &mut config, "-123");
        assert!(resp.contains("MentionOnly"));
    }
}
