use super::{FunctionDef, ToolDefinition};

/// The bash tool definition.
pub(crate) fn bash_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "bash".into(),
            description: "Execute a bash command in the container. Use for file operations, API calls, invoking skills, and reading raw files. Commands are stateless and oneshot.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute."
                    }
                },
                "required": ["command"]
            }),
        },
    }
}

/// The read_memory tool definition.
pub(crate) fn read_memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "read_memory".into(),
            description: "Read a user's memory file. Returns the full memory as JSON with frontmatter fields (user_id, username, call_name, description) and body (log entries).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "user_id": {
                        "type": "string",
                        "description": "The Telegram user ID whose memory to read."
                    }
                },
                "required": ["user_id"]
            }),
        },
    }
}

/// The update_memory tool definition.
pub(crate) fn update_memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "update_memory".into(),
            description: "Update a user's memory file. All fields are optional — only provided fields are overwritten. Use log_entry to append a timestamped line to the body. Use call_name to set what you should call them. Use description to update your summary of the user.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "user_id": {
                        "type": "string",
                        "description": "The Telegram user ID whose memory to update."
                    },
                    "username": {
                        "type": "string",
                        "description": "Optional: update the Telegram @username."
                    },
                    "call_name": {
                        "type": "string",
                        "description": "Optional: update what you call this user."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional: update the summary/description of this user."
                    },
                    "log_entry": {
                        "type": "string",
                        "description": "Optional: a fact or event to append as a timestamped log entry."
                    }
                },
                "required": ["user_id"]
            }),
        },
    }
}

/// All tool definitions. When `include_bash` is false, the bash tool is excluded.
/// When `model` is Some, adds the search_conversations RAG tool.
/// `media_dir` is the configured media directory path, shown in the send_media description.
/// `image_gen_model` enables the `generate_image` tool.
/// `image_fallback_model` enables the `describe_image` tool.
/// `advice_model` enables the `ask_advisor` tool.
pub fn all_tool_definitions(
    include_bash: bool,
    embedding_model: Option<&str>,
    media_dir: &str,
    image_gen_model: Option<&str>,
    image_fallback_model: Option<&str>,
    advice_model: Option<&str>,
) -> Vec<ToolDefinition> {
    let mut tools = vec![
        read_memory_tool_definition(),
        update_memory_tool_definition(),
        read_chat_memory_tool_definition(),
        update_chat_memory_tool_definition(),
        create_skill_tool_definition(),
        read_skill_tool_definition(),
        update_skill_tool_definition(),
        add_task_tool_definition(),
        list_tasks_tool_definition(),
        remove_task_tool_definition(),
        create_reminder_tool_definition(),
        list_reminders_tool_definition(),
        remove_reminder_tool_definition(),
        create_todo_tool_definition(),
        list_todos_tool_definition(),
        edit_todo_tool_definition(),
        delete_todo_tool_definition(),
        get_recent_messages_tool_definition(),
        send_message_tool_definition(),
    ];
    if embedding_model.is_some() {
        tools.push(search_conversations_tool_definition());
    }
    tools.push(list_media_tool_definition(media_dir));
    tools.push(send_media_tool_definition(media_dir));
    if image_gen_model.is_some() {
        tools.push(generate_image_tool_definition(media_dir));
    }
    if image_fallback_model.is_some() {
        tools.push(describe_image_tool_definition());
    }
    if advice_model.is_some() {
        tools.push(ask_advisor_tool_definition(advice_model.unwrap()));
    }
    if include_bash {
        tools.insert(0, bash_tool_definition());
    }
    // Config tools are always available (they have their own permission model)
    tools.push(read_config_schema_tool_definition());
    tools.push(read_config_tool_definition());
    tools.push(edit_config_tool_definition());
    // Model info tools are always available
    tools.push(get_model_info_tool_definition());
    tools.push(propose_model_change_tool_definition());
    tools
}

/// The read_chat_memory tool definition.
pub(crate) fn read_chat_memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "read_chat_memory".into(),
            description: "Read the chat-level memory for this conversation. Returns JSON with call_name, description, and body (log entries). Use this to recall context about the chat itself — topics, participants, group purpose, etc.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The update_chat_memory tool definition.
pub(crate) fn update_chat_memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "update_chat_memory".into(),
            description: "Update the chat-level memory. All fields optional — only provided fields are overwritten. Use call_name to name the chat, description to summarize it, log_entry to append a timestamped fact.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "call_name": {
                        "type": "string",
                        "description": "Optional: a name for this chat (e.g. 'Rust Study Group')."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional: summary of what this chat is about."
                    },
                    "log_entry": {
                        "type": "string",
                        "description": "Optional: a fact to append as a timestamped log entry."
                    }
                },
                "required": []
            }),
        },
    }
}

/// The create_skill tool definition.
pub(crate) fn create_skill_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "create_skill".into(),
            description: "Create a new skill. Skills extend my capabilities with shell commands, API calls, or custom workflows. Each skill gets a directory under skills/<name>/ with a skill.md file.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique skill name (lowercase, hyphens for spaces, e.g. 'search-web')."
                    },
                    "description": {
                        "type": "string",
                        "description": "Short description of what the skill does."
                    },
                    "body": {
                        "type": "string",
                        "description": "Instructions for using the skill — bash commands, API endpoints, workflow steps. Gets injected into my system prompt."
                    }
                },
                "required": ["name", "description", "body"]
            }),
        },
    }
}

/// The read_skill tool definition.
pub(crate) fn read_skill_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "read_skill".into(),
            description: "Read an existing skill's full content. Returns JSON with name, description, and body. Use before updating a skill so you know what's already there.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to read."
                    }
                },
                "required": ["name"]
            }),
        },
    }
}

/// The update_skill tool definition.
pub(crate) fn update_skill_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "update_skill".into(),
            description: "Update an existing skill. Only provided fields are overwritten. Skills are reloaded automatically after updating.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the existing skill to update."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional: new description."
                    },
                    "body": {
                        "type": "string",
                        "description": "Optional: new body/instructions."
                    }
                },
                "required": ["name"]
            }),
        },
    }
}

/// The add_task tool definition.
pub(crate) fn add_task_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "add_task".into(),
            description: "Add a task to this chat's task list. The bot will work on tasks autonomously on a heartbeat timer.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "What needs to be done. Be specific and actionable."
                    }
                },
                "required": ["description"]
            }),
        },
    }
}

/// The list_tasks tool definition.
pub(crate) fn list_tasks_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "list_tasks".into(),
            description: "List all pending tasks for this chat.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The remove_task tool definition.
pub(crate) fn remove_task_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "remove_task".into(),
            description: "Remove a completed or obsolete task from this chat's task list.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The ID of the task to remove (from list_tasks)."
                    }
                },
                "required": ["id"]
            }),
        },
    }
}

/// The create_reminder tool definition.
pub(crate) fn create_reminder_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "create_reminder".into(),
            description: "Create a reminder that will fire at a specific time. Use this when a user asks to be reminded about something at a known time (e.g. 'remind me tomorrow at 3pm to call mom'). The trigger_at must be an ISO 8601 timestamp in UTC. Optionally provide an action for the bot to perform when the reminder fires (e.g. looking up information first). If the request has no specific time or depends on external state (e.g. 'tell me when the stock hits $100'), use add_task instead.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "What to remind the user about (e.g. 'Call your mom'). This is sent as-is when the reminder fires."
                    },
                    "trigger_at": {
                        "type": "string",
                        "description": "ISO 8601 timestamp in UTC when the reminder should fire (e.g. '2026-05-11T18:00:00Z'). Convert the user's natural language time to UTC."
                    },
                    "action": {
                        "type": "string",
                        "description": "Optional: what the bot should do when the reminder fires, beyond just sending the description. For example: 'Look up mom's phone number from memory and include it in the message'. If omitted, only the description is sent."
                    }
                },
                "required": ["description", "trigger_at"]
            }),
        },
    }
}

/// The list_reminders tool definition.
pub(crate) fn list_reminders_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "list_reminders".into(),
            description: "List all pending reminders for this chat, including their trigger times.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The remove_reminder tool definition.
pub(crate) fn remove_reminder_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "remove_reminder".into(),
            description: "Remove a pending reminder from this chat's reminder list.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The ID of the reminder to remove (from list_reminders)."
                    }
                },
                "required": ["id"]
            }),
        },
    }
}

/// The create_todo tool definition.
pub(crate) fn create_todo_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "create_todo".into(),
            description: "Add a todo item to the user's todo list. Use this when a user asks to remember something, track a task, or add something to their todo list. The todo is for the human to complete — not for the bot.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "The todo item description. Be clear and specific about what needs to be done."
                    }
                },
                "required": ["description"]
            }),
        },
    }
}

/// The list_todos tool definition.
pub(crate) fn list_todos_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "list_todos".into(),
            description: "List all todos for this chat, showing their ID, description, completed status, and timestamps. Use this to show the user what's on their todo list.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The edit_todo tool definition.
pub(crate) fn edit_todo_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "edit_todo".into(),
            description: "Edit an existing todo item by its UUID. You can update the description or toggle its completed status. Use this when the user wants to modify a todo, mark it as done, or mark it as not done.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The UUID of the todo to edit (from list_todos)."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional: new description for the todo. If omitted, the description is unchanged."
                    },
                    "completed": {
                        "type": "boolean",
                        "description": "Optional: set completed status. true = mark as done, false = mark as not done. If omitted, the status is unchanged."
                    }
                },
                "required": ["id"]
            }),
        },
    }
}

/// The delete_todo tool definition.
pub(crate) fn delete_todo_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "delete_todo".into(),
            description: "Delete a todo item by its UUID. Use this when the user wants to remove a todo completely (as opposed to marking it completed).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The UUID of the todo to delete (from list_todos)."
                    }
                },
                "required": ["id"]
            }),
        },
    }
}

/// The get_recent_messages tool definition.
pub(crate) fn get_recent_messages_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "get_recent_messages".into(),
            description: "Get the last N messages from this conversation. Call this when you need to recall something from earlier in the chat. Returns a JSON array of recent messages with role, content, and sender name.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "count": {
                        "type": "integer",
                        "description": "Number of recent messages to retrieve (default: 10, max: 50)"
                    }
                },
                "required": []
            }),
        },
    }
}

/// The send_message tool definition.
pub(crate) fn send_message_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "send_message".into(),
            description: "Send a plain text message to the current chat. In normal conversations, use this ONLY for headsup/intermediate messages (e.g. 'ok, give me a second, taking a look now...') — never for your final answer, which is sent automatically. In background tasks, use this once to report completion or deliver results. Use sparingly.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The plain text message to send."
                    }
                },
                "required": ["text"]
            }),
        },
    }
}

/// The send_media tool definition.
pub(crate) fn send_media_tool_definition(media_dir: &str) -> ToolDefinition {
    let desc = format!(
        "Send a media file to the current chat. Use this to upload files generated via bash or MCP tools — screenshots, plots, logfiles, exports, downloads, etc. Auto-detects media type from file extension (photos, videos, audio, documents). \
         File paths can be relative to the data directory, absolute, or inside the media directory at '{}'.",
        media_dir
    );
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "send_media".into(),
            description: desc,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the media file. Relative paths are resolved against the data directory."
                    },
                    "caption": {
                        "type": "string",
                        "description": "Optional caption to send with the media."
                    },
                    "original_quality": {
                        "type": "boolean",
                        "description": "If true, send the file as a document to preserve original quality without compression or resize. Default: false."
                    }
                },
                "required": ["file_path"]
            }),
        },
    }
}

/// The list_media tool definition.
pub(crate) fn list_media_tool_definition(media_dir: &str) -> ToolDefinition {
    let desc = format!(
        "List the contents of the media directory at '{}' and its subdirectories recursively. \
         Returns a tree of files and directories with relative paths. \
         Use this to browse available media files before deciding which to send with send_media. \
         Results are truncated at 500 entries to avoid overwhelming output.",
        media_dir
    );
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "list_media".into(),
            description: desc,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "subpath": {
                        "type": "string",
                        "description": "Optional: a subdirectory path relative to the media directory to list (e.g. 'images/cats'). If omitted, lists the root media directory."
                    }
                },
                "required": []
            }),
        },
    }
}

/// The generate_image tool definition.
/// Only exposed when `image_gen_model` is configured.
pub(crate) fn generate_image_tool_definition(media_dir: &str) -> ToolDefinition {
    let desc = format!(
        "Generate images from a text description via OpenRouter chat completions. Optionally provide reference images to guide style, composition, or content. Generated images are saved to the media directory at '{}' and their absolute file paths are returned — use send_media to display them. Reference images must be file paths to existing images (PNG, JPEG, WebP, GIF).",
        media_dir
    );
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "generate_image".into(),
            description: desc,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Text description of the image to generate. Be detailed — describe subject, style, lighting, composition, colors, mood."
                    },
                    "size": {
                        "type": "string",
                        "description": "Aspect ratio (e.g. '16:9', '1:1', '4:3') or image size (e.g. '1K', '2K', '4K'). Aspect ratios are preferred for most models; use image size only if the model supports it."
                    },
                    "reference_images": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional file paths to reference images to guide the generation (style, composition, etc.). Paths can be relative to the data directory, absolute, or inside the media directory."
                    }
                },
                "required": ["prompt"]
            }),
        },
    }
}

/// The read_config_schema tool definition.
pub(crate) fn read_config_schema_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "read_config_schema".into(),
            description: "Get the JSON Schema describing all available configuration fields, their types, descriptions, and defaults. Use this BEFORE calling read_config or edit_config to understand what fields exist and what values are allowed. Returns the complete schema for Config, ChatConfig, DmConfig, OpenRouterConfig, and all nested types.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The read_config tool definition.
pub(crate) fn read_config_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "read_config".into(),
            description: "Read the current bot configuration. Returns the full config.yaml as a YAML string. Always call this first before using edit_config so you can see the current settings.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The edit_config tool definition.
pub(crate) fn edit_config_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "edit_config".into(),
            description: "Propose changes to the bot configuration. Provide the COMPLETE new config as YAML (call read_config first, then modify and provide the full result). The diff will be shown to the user for approval with Accept/Deny buttons. If the YAML is invalid, an error is returned. If valid, the proposal is sent to the user and you should wait for their response before asking about config changes.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "config_yaml": {
                        "type": "string",
                        "description": "The complete new config.yaml content as a YAML string. Must be valid YAML that can be parsed as a Config."
                    }
                },
                "required": ["config_yaml"]
            }),
        },
    }
}

/// The search_conversations tool definition (RAG).
/// Only exposed when an embedding model is configured.
pub(crate) fn search_conversations_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "search_conversations".into(),
            description: "Search past conversation messages by semantic similarity. Use this to find relevant messages from earlier in the chat — e.g. 'that discussion about docker volumes' or 'what did Alice say about the deadline?'. Returns ranked results with similarity scores and content.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to search for — describe the topic, question, or phrase you're looking for."
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of results to return (default: 5, max: 10)"
                    }
                },
                "required": ["query"]
            }),
        },
    }
}

/// The get_model_info tool definition.
pub(crate) fn get_model_info_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "get_model_info".into(),
            description: "Get information about the currently active model for this chat. Returns the effective model (including any routing specifier like :nitro, :floor, :free), the config default model, whether there's a temporary override, the model's context length and pricing (if known from OpenRouter metadata), and the list of available routing specifiers. Use this when the user asks about the current model, or before proposing a model change so you know what's currently set.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The ask_advisor tool definition.
/// When an advice model is configured, the LLM can call this tool to get a second opinion
/// from a larger/smarter model. The tool sends the query plus recent conversation context
/// to the advice model and returns its response.
pub(crate) fn ask_advisor_tool_definition(advice_model: &str) -> ToolDefinition {
    let desc = format!(
        "Ask for a second opinion or advice from a more capable model ({}). Use this when you need deeper analysis, a second perspective, or want to leverage a larger model's reasoning on a complex question. Provide a clear, self-contained query and optionally specify how many recent messages of conversation context to include.",
        advice_model
    );
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "ask_advisor".into(),
            description: desc,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The question or problem to ask the advisor model. Be clear and self-contained."
                    }
                },
                "required": ["query"]
            }),
        },
    }
}

/// The describe_image tool definition.
/// Only exposed when `image_fallback_model` is configured.
pub(crate) fn describe_image_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "describe_image".into(),
            description: "Get a detailed description of an image using a vision-capable model. Before calling, send a brief heads-up via send_message to let the user know you're analyzing the image (e.g. 'Let me take a closer look...') — this may take a moment. Use this tool when you need specific visual details that aren't obvious from context — e.g. portion sizes, text reading, object identification, calorie estimation, spatial layout, etc. The image is already saved to disk; provide the file_path from the user's message metadata and a specific prompt requesting exactly what details you need.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the image file on disk. This is provided in the user's message when they send an image."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "What to ask about the image. Be specific — request measurements, text reading, object identification, or any other details you need."
                    }
                },
                "required": ["file_path", "prompt"]
            }),
        },
    }
}

/// The propose_model_change tool definition.
pub(crate) fn propose_model_change_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "propose_model_change".into(),
            description: "Propose a temporary model change to the user. This sends an Accept/Deny dialog to the chat. If the user accepts, the model is temporarily switched (until /model_default resets it). If denied, nothing changes. Use this when the user asks to try a different model, or when you recommend a model switch. You can provide a model_id and/or a routing specifier (:nitro, :floor, :free) to apply to the current model. Returns a status indicating the proposal was sent — DO NOT ask the user again about it, wait for their button response.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "model_id": {
                        "type": "string",
                        "description": "The full model ID to switch to (e.g. 'openai/gpt-4o', 'anthropic/claude-sonnet-4'). If omitted and specifier is provided, the specifier is applied to the current model."
                    },
                    "specifier": {
                        "type": "string",
                        "description": "Optional routing specifier to apply: 'nitro', 'floor', or 'free'. If model_id is provided, the specifier is appended to it. If only specifier is provided, it's applied to the current model."
                    }
                },
                "required": []
            }),
        },
    }
}
