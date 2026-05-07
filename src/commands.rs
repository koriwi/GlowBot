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
    /// /run — trigger heartbeat/task agent immediately
    Run,
    /// /new — set a "forget" cutoff; only messages after this point are included in context
    New,
    /// /prompt — show the full system prompt and conversation history that would be sent to the LLM
    Prompt,
}

/// Parse a Telegram message to see if it's a bot command.
/// Strips `@botname` suffix (Telegram adds it when multiple bots share a command).
pub fn parse_command(text: &str) -> Option<Command> {
    let text = text.trim();
    if !text.starts_with('/') {
        return None;
    }

    let (cmd, _args) = match text.split_once(' ') {
        Some((c, a)) => (c.trim(), a.trim()),
        None => (text, ""),
    };

    // Strip @botname suffix (e.g. /tasks@glowythebot → /tasks)
    let cmd = cmd.split_once('@').map(|(base, _)| base).unwrap_or(cmd);

    match cmd {
        "/status" => Some(Command::Status),
        "/stop" => Some(Command::Stop),
        "/tasks" => Some(Command::Tasks),
        "/run" => Some(Command::Run),
        "/new" => Some(Command::New),
        "/prompt" => Some(Command::Prompt),
        _ => None,
    }
}

/// Check if a user is allowed to run bot commands in a group chat.
/// Empty command_whitelist = nobody can run commands.
pub fn can_run_command(chat_config: &ChatConfig, user_id: &str) -> bool {
    if chat_config.command_whitelist.is_empty() {
        return false;
    }
    chat_config
        .command_whitelist
        .contains(&user_id.to_string())
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
            let model = config.model_for_chat(chat_id);
            if chat_id.starts_with('-') {
                // Group chat
                let chat = config.chat_config(chat_id);
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
            } else {
                // DM
                let dm = config.dm_config(chat_id);
                format!(
                    "Chat ID: {}\nModel: {}\nContext usage: {}\nDM commands: {}\nDM system prompt: {}",
                    chat_id,
                    model,
                    context_usage,
                    dm.map(|d| if d.commands_enabled { "enabled" } else { "disabled" })
                        .unwrap_or("disabled (no DM config)"),
                    dm.map(|d| {
                        if d.system_prompt.is_empty() {
                            "not set".to_string()
                        } else {
                            format!("set ({} chars)", d.system_prompt.len())
                        }
                    })
                    .unwrap_or("not set (no DM config)".to_string()),
                )
            }
        }
        Command::Stop => "Stop command received.".to_string(),
        Command::Tasks => String::new(), // handled in handle_bot_command
        Command::Run => String::new(),   // handled in handle_bot_command
        Command::New => String::new(),   // handled in handle_bot_command
        Command::Prompt => String::new(), // handled in handle_bot_command
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
    fn test_parse_command_strips_botname_suffix() {
        assert_eq!(parse_command("/tasks@glowythebot"), Some(Command::Tasks));
        assert_eq!(parse_command("/status@otherbot"), Some(Command::Status));
        assert_eq!(parse_command("/stop@somebot"), Some(Command::Stop));
        // With arguments
        assert_eq!(
            parse_command("/tasks@glowythebot extra"),
            Some(Command::Tasks)
        );
    }

    #[test]
    fn test_parse_command_run() {
        assert_eq!(parse_command("/run"), Some(Command::Run));
        assert_eq!(parse_command("/run@glowythebot"), Some(Command::Run));
    }

    #[test]
    fn test_parse_command_new() {
        assert_eq!(parse_command("/new"), Some(Command::New));
        assert_eq!(parse_command("/new@glowythebot"), Some(Command::New));
    }

    #[test]
    fn test_parse_command_prompt() {
        assert_eq!(parse_command("/prompt"), Some(Command::Prompt));
        assert_eq!(parse_command("/prompt@glowythebot"), Some(Command::Prompt));
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
        // Empty command_whitelist → nobody can run
        let config = ChatConfig::default();
        assert!(!can_run_command(&config, "123"));

        // Whitelist set → only listed users can run
        let config = ChatConfig {
            command_whitelist: vec!["456".into()],
            ..Default::default()
        };
        assert!(can_run_command(&config, "456"));
        assert!(!can_run_command(&config, "789"));
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
    fn test_handle_status_command_group() {
        let mut config = crate::config::basic_config();
        config.openrouter.model = "default-model".into();
        let resp = handle_command(&Command::Status, &mut config, "-123", "1k/10k (10%)");
        assert!(resp.contains("-123"));
        assert!(resp.contains("default-model"));
        assert!(resp.contains("1k/10k (10%)"));
        assert!(resp.contains("MentionOnly"));
        assert!(resp.contains("everyone")); // interaction whitelist
        assert!(resp.contains("nobody")); // command whitelist empty = nobody
        // Group chat must not contain DM-specific fields
        assert!(!resp.contains("DM commands"));
    }

    #[test]
    fn test_handle_status_command_dm_registered() {
        let mut config = crate::config::basic_config();
        config.openrouter.model = "default-model".into();
        config.dms.insert(
            "123456".into(),
            crate::config::DmConfig {
                model: Some("dm-model".into()),
                commands_enabled: true,
                system_prompt: "You are helpful.".into(),
                ..Default::default()
            },
        );
        let resp = handle_command(&Command::Status, &mut config, "123456", "2k/20k (10%)");
        assert!(resp.contains("123456"));
        assert!(resp.contains("dm-model"));
        assert!(resp.contains("2k/20k (10%)"));
        assert!(resp.contains("DM commands: enabled"));
        assert!(resp.contains("DM system prompt: set (16 chars)"));
        // DM must not contain group-specific fields
        assert!(!resp.contains("Interaction mode"));
        assert!(!resp.contains("Interaction whitelist"));
        assert!(!resp.contains("Command whitelist"));
    }

    #[test]
    fn test_handle_status_command_dm_unregistered() {
        let mut config = crate::config::basic_config();
        config.openrouter.model = "default-model".into();
        // No DM config for this chat_id — not in the dms map
        let resp = handle_command(&Command::Status, &mut config, "999", "5k/100k (5%)");
        assert!(resp.contains("999"));
        assert!(resp.contains("default-model"));
        assert!(resp.contains("5k/100k (5%)"));
        assert!(resp.contains("DM commands: disabled (no DM config)"));
        assert!(resp.contains("DM system prompt: not set (no DM config)"));
        assert!(!resp.contains("Interaction mode"));
    }

    #[test]
    fn test_handle_stop_command() {
        let mut config = crate::config::basic_config();
        let resp = handle_command(&Command::Stop, &mut config, "-123", "");
        assert!(resp.contains("Stop command received"));
    }
}
