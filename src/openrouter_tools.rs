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
pub fn all_tool_definitions(
    include_bash: bool,
    embedding_model: Option<&str>,
    media_dir: &str,
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
        get_recent_messages_tool_definition(),
        send_message_tool_definition(),
    ];
    if embedding_model.is_some() {
        tools.push(search_conversations_tool_definition());
    }
    tools.push(list_media_tool_definition(media_dir));
    tools.push(send_media_tool_definition(media_dir));
    if include_bash {
        tools.insert(0, bash_tool_definition());
    }
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
