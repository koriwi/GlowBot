use crate::config::{ChatConfig, Config};

/// Result of parsing a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// /status
    Status,
    /// /codex_usage — show Codex subscription allowance and reset times
    CodexUsage,
    /// /stop
    Stop,
    /// /tasks
    Tasks,
    /// /todos — human todo list. With "details" argument: shows IDs and timestamps.
    Todos(bool),
    /// /reminders — list pending reminders
    Reminders,
    /// /run — trigger heartbeat/task agent immediately
    Run,
    /// /new — set a "forget" cutoff; only messages after this point are included in context
    New,
    /// /prompt — show the full system prompt and conversation history that would be sent to the LLM
    Prompt,
    /// /tools — list all tools available in this chat
    Tools,
    /// /config — show the current config (sensitive fields redacted)
    Config,
    /// /config_schema — show the JSON Schema for all config fields
    ConfigSchema,
    /// /models — browse and temporarily switch models via inline keyboard
    Models,
    /// /model_default (alias /model_reset) — reset the model to config default
    ModelDefault,
    /// /model [model-id|:specifier] — set or view the model
    Model(Option<String>),
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
        "/codex_usage" => Some(Command::CodexUsage),
        "/stop" => Some(Command::Stop),
        "/tasks" => Some(Command::Tasks),
        "/todos" => {
            let args = text.split_once(' ').map(|(_, a)| a.trim().to_lowercase());
            let details = args.as_deref() == Some("details");
            Some(Command::Todos(details))
        }
        "/reminders" => Some(Command::Reminders),
        "/run" => Some(Command::Run),
        "/new" => Some(Command::New),
        "/prompt" => Some(Command::Prompt),
        "/tools" => Some(Command::Tools),
        "/config" => Some(Command::Config),
        "/config_schema" => Some(Command::ConfigSchema),
        "/models" => Some(Command::Models),
        "/model_default" | "/model_reset" => Some(Command::ModelDefault),
        "/model" => {
            let args = text.split_once(' ').map(|(_, a)| a.trim().to_string());
            Some(Command::Model(args.filter(|a| !a.is_empty())))
        }
        _ => None,
    }
}

/// Check if a user is allowed to run bot commands in a group chat.
/// Empty command_whitelist = nobody can run commands.
pub fn can_run_command(chat_config: &ChatConfig, user_id: &str) -> bool {
    if chat_config.command_whitelist.is_empty() {
        return false;
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
    handle_command_with_model(command, config, chat_id, context_usage, None, None)
}

/// Handle a command with an optional effective model override (for /status showing
/// temporary model overrides set via inline keyboard).
pub fn handle_command_with_model(
    command: &Command,
    config: &mut Config,
    chat_id: &str,
    context_usage: &str,
    effective_model: Option<&str>,
    effective_provider: Option<crate::config::LlmProvider>,
) -> String {
    match command {
        Command::Status => {
            let model = effective_model.unwrap_or_else(|| config.model_for_chat(chat_id));
            let provider = format!(
                "{:?}",
                effective_provider.unwrap_or_else(|| config.provider_for_chat(chat_id))
            )
            .to_lowercase();
            if chat_id.starts_with('-') {
                // Group chat
                let chat = config.chat_config(chat_id);
                let name_line = chat
                    .name
                    .as_ref()
                    .map(|n| format!("Name: {}\n", n))
                    .unwrap_or_default();
                format!(
                    "{}Chat ID: {}\nProvider: {}\nModel: {}\nContext usage: {}\nInteraction mode: {:?}\nInteraction whitelist: {}\nCommand whitelist: {}",
                    name_line,
                    chat_id,
                    provider,
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
                let name_line = dm
                    .and_then(|d| d.name.as_ref())
                    .map(|n| format!("Name: {}\n", n))
                    .unwrap_or_default();
                format!(
                    "{}Chat ID: {}\nProvider: {}\nModel: {}\nContext usage: {}\nDM commands: {}\nDM system prompt: {}",
                    name_line,
                    chat_id,
                    provider,
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
        Command::CodexUsage => String::new(), // handled in handle_bot_command
        Command::Stop => "Stop command received.".to_string(),
        Command::Tasks => String::new(), // handled in handle_bot_command
        Command::Todos(_) => String::new(), // handled in handle_bot_command
        Command::Reminders => String::new(), // handled in handle_bot_command
        Command::Run => String::new(),   // handled in handle_bot_command
        Command::New => String::new(),   // handled in handle_bot_command
        Command::Prompt => String::new(), // handled in handle_bot_command
        Command::Tools => String::new(), // handled in handle_bot_command
        Command::Config => String::new(), // handled in handle_bot_command
        Command::ConfigSchema => String::new(), // handled in handle_bot_command
        Command::Models => String::new(), // handled in handle_bot_command
        Command::ModelDefault => String::new(), // handled in handle_bot_command
        Command::Model(_) => String::new(), // handled in handle_bot_command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_status() {
        assert_eq!(parse_command("/status"), Some(Command::Status));
        assert_eq!(parse_command("/codex_usage"), Some(Command::CodexUsage));
        assert_eq!(
            parse_command("/codex_usage@glowythebot"),
            Some(Command::CodexUsage)
        );
    }

    #[test]
    fn test_parse_command_stop() {
        assert_eq!(parse_command("/stop"), Some(Command::Stop));
    }

    #[test]
    fn test_parse_command_todos() {
        assert_eq!(parse_command("/todos"), Some(Command::Todos(false)));
        assert_eq!(parse_command("/todos details"), Some(Command::Todos(true)));
        assert_eq!(parse_command("/todos DETAILS"), Some(Command::Todos(true)));
        assert_eq!(
            parse_command("/todos@glowythebot"),
            Some(Command::Todos(false))
        );
        assert_eq!(
            parse_command("/todos@glowythebot details"),
            Some(Command::Todos(true))
        );
        // other args are not "details"
        assert_eq!(parse_command("/todos foo"), Some(Command::Todos(false)));
    }

    #[test]
    fn test_parse_command_strips_botname_suffix() {
        assert_eq!(
            parse_command("/todos@glowythebot"),
            Some(Command::Todos(false))
        );
        assert_eq!(parse_command("/tasks@glowythebot"), Some(Command::Tasks));
        assert_eq!(
            parse_command("/reminders@glowythebot"),
            Some(Command::Reminders)
        );
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
    fn test_parse_command_reminders() {
        assert_eq!(parse_command("/reminders"), Some(Command::Reminders));
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
    fn test_parse_command_tools() {
        assert_eq!(parse_command("/tools"), Some(Command::Tools));
        assert_eq!(parse_command("/tools@glowythebot"), Some(Command::Tools));
    }

    #[test]
    fn test_parse_command_config() {
        assert_eq!(parse_command("/config"), Some(Command::Config));
        assert_eq!(parse_command("/config@glowythebot"), Some(Command::Config));
    }

    #[test]
    fn test_parse_command_config_schema() {
        assert_eq!(parse_command("/config_schema"), Some(Command::ConfigSchema));
        assert_eq!(
            parse_command("/config_schema@glowythebot"),
            Some(Command::ConfigSchema)
        );
    }

    #[test]
    fn test_parse_command_models() {
        assert_eq!(parse_command("/models"), Some(Command::Models));
        assert_eq!(parse_command("/models@glowythebot"), Some(Command::Models));
    }

    #[test]
    fn test_parse_command_model_default() {
        assert_eq!(parse_command("/model_default"), Some(Command::ModelDefault));
        assert_eq!(parse_command("/model_reset"), Some(Command::ModelDefault));
        assert_eq!(
            parse_command("/model_default@glowythebot"),
            Some(Command::ModelDefault)
        );
        assert_eq!(
            parse_command("/model_reset@glowythebot"),
            Some(Command::ModelDefault)
        );
    }

    #[test]
    fn test_parse_not_a_command() {
        assert!(parse_command("Hello!").is_none());
        assert!(parse_command("").is_none());
        assert!(parse_command("   hi   ").is_none());
        // /model is now a command
        assert_eq!(parse_command("/model"), Some(Command::Model(None)));
        assert_eq!(
            parse_command("/model gpt-4"),
            Some(Command::Model(Some("gpt-4".into())))
        );
        assert_eq!(
            parse_command("/model :nitro"),
            Some(Command::Model(Some(":nitro".into())))
        );
        assert_eq!(
            parse_command("/model foo/bar:nitro"),
            Some(Command::Model(Some("foo/bar:nitro".into())))
        );
        assert_eq!(
            parse_command("/model@glowythebot :floor"),
            Some(Command::Model(Some(":floor".into())))
        );
        // /mode and /reload are no longer commands
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
        // Name field should not appear when not set
        assert!(!resp.contains("Name:"));
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
        // Name field should not appear when not set
        assert!(!resp.contains("Name:"));
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
        // Name field should not appear when not set
        assert!(!resp.contains("Name:"));
    }

    #[test]
    fn test_handle_status_with_name() {
        let mut config = crate::config::basic_config();
        config.openrouter.model = "default-model".into();

        // Group chat with name
        config.chats.insert(
            "-123".into(),
            crate::config::ChatConfig {
                name: Some("Team Chat".into()),
                ..Default::default()
            },
        );
        let resp = handle_command(&Command::Status, &mut config, "-123", "1k/10k (10%)");
        assert!(resp.contains("Name: Team Chat"));

        // DM with name
        config.dms.insert(
            "123456".into(),
            crate::config::DmConfig {
                name: Some("Alice".into()),
                commands_enabled: true,
                ..Default::default()
            },
        );
        let resp = handle_command(&Command::Status, &mut config, "123456", "2k/20k (10%)");
        assert!(resp.contains("Name: Alice"));
    }

    #[test]
    fn test_handle_stop_command() {
        let mut config = crate::config::basic_config();
        let resp = handle_command(&Command::Stop, &mut config, "-123", "");
        assert!(resp.contains("Stop command received"));
    }
}
