use serde::{Deserialize, Serialize};

/// Filename for per-chat memory (as opposed to per-user).
pub const CHAT_MEMORY_FILE: &str = "_chat.md";

/// Structured metadata stored in the YAML frontmatter of memory files.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryFrontmatter {
    /// Telegram user ID.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user_id: String,
    /// Telegram username.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    /// What the bot calls this user.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub call_name: String,
    /// A short description / summary about the user.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A full memory file: frontmatter + body (log entries).
#[derive(Debug, Clone)]
pub struct Memory {
    pub frontmatter: MemoryFrontmatter,
    pub body: String,
}

impl Memory {
    /// Create an empty memory for a user.
    pub fn new(user_id: &str, username: &str) -> Self {
        Self {
            frontmatter: MemoryFrontmatter {
                user_id: user_id.to_string(),
                username: username.to_string(),
                ..Default::default()
            },
            body: String::new(),
        }
    }

    /// Create an empty chat-level memory.
    pub fn new_chat() -> Self {
        Self {
            frontmatter: MemoryFrontmatter {
                user_id: "_chat".into(),
                ..Default::default()
            },
            body: String::new(),
        }
    }

    /// Serialize to the memory file format (YAML frontmatter + Markdown body).
    pub fn to_file_content(&self) -> String {
        let yaml = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let body = if self.body.is_empty() {
            String::new()
        } else {
            format!("\n{}", self.body)
        };
        format!("---\n{}---{}", yaml, body)
    }

    /// Append a timestamped log entry to the body.
    pub fn append_log(&mut self, entry: &str) {
        let timestamp = chrono::Utc::now().format("%Y-%m-%d");
        if !self.body.is_empty() {
            self.body.push('\n');
        }
        self.body.push_str(&format!("- {}: {}", timestamp, entry));
    }

    /// Generate the system prompt fragment injected into context.
    /// Includes frontmatter fields and full body (log entries).
    pub fn to_system_prompt(&self) -> String {
        let fm = &self.frontmatter;
        let mut parts = vec![format!("User ID: {}", fm.user_id)];
        if !fm.username.is_empty() {
            parts.push(format!("Username: {}", fm.username));
        }
        if !fm.call_name.is_empty() {
            parts.push(format!("Call them: {}", fm.call_name));
        }
        if !fm.description.is_empty() {
            parts.push(format!("About them: {}", fm.description));
        }
        if !self.body.is_empty() {
            parts.push(format!("Memory log:\n{}", self.body));
        }
        parts.join("\n")
    }

    /// Generate the system prompt fragment for a chat-level memory.
    /// Includes frontmatter fields and full body (log entries).
    pub fn to_chat_system_prompt(&self) -> String {
        let fm = &self.frontmatter;
        let mut parts = vec!["This chat:".to_string()];
        if !fm.call_name.is_empty() {
            parts.push(format!("- Name: {}", fm.call_name));
        }
        if !fm.description.is_empty() {
            parts.push(format!("- About: {}", fm.description));
        }
        if !self.body.is_empty() {
            parts.push(format!("- History:\n{}", self.body));
        }
        if parts.len() == 1 {
            String::new()
        } else {
            parts.join("\n")
        }
    }
}

/// Parse a memory file from its content string.
pub fn parse_memory(content: &str) -> Option<Memory> {
    let (frontmatter_str, body) = crate::skills::parse_frontmatter(content)?;
    let frontmatter: MemoryFrontmatter = serde_yaml::from_str(frontmatter_str).ok()?;
    Some(Memory {
        frontmatter,
        body: body.to_string(),
    })
}

/// Load a user's memory from disk.
pub fn load_memory(chats_dir: &std::path::Path, chat_id: &str, user_id: &str) -> Option<Memory> {
    let path = chats_dir.join(chat_id).join(format!("{}.md", user_id));
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    parse_memory(&content)
}

/// Load the chat-level memory from disk.
pub fn load_chat_memory(chats_dir: &std::path::Path, chat_id: &str) -> Option<Memory> {
    let path = chats_dir.join(chat_id).join(CHAT_MEMORY_FILE);
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    parse_memory(&content)
}

/// Save a user's memory to disk.
pub fn save_memory(
    chats_dir: &std::path::Path,
    chat_id: &str,
    user_id: &str,
    memory: &Memory,
) -> anyhow::Result<()> {
    let dir = chats_dir.join(chat_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.md", user_id));
    std::fs::write(&path, memory.to_file_content())?;
    Ok(())
}

/// Save the chat-level memory to disk.
pub fn save_chat_memory(
    chats_dir: &std::path::Path,
    chat_id: &str,
    memory: &Memory,
) -> anyhow::Result<()> {
    let dir = chats_dir.join(chat_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(CHAT_MEMORY_FILE);
    std::fs::write(&path, memory.to_file_content())?;
    Ok(())
}

/// Load memories for all known users in a chat (excludes chat-level `_chat.md`).
pub fn load_chat_memories(
    chats_dir: &std::path::Path,
    chat_id: &str,
) -> anyhow::Result<Vec<Memory>> {
    let dir = chats_dir.join(chat_id);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut memories = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name == CHAT_MEMORY_FILE {
            continue; // skip chat-level memory
        }
        if path.extension().is_some_and(|e| e == "md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Some(memory) = parse_memory(&content) {
                    memories.push(memory);
                }
            }
        }
    }
    Ok(memories)
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
