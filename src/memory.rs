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
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_memory_new() {
        let mem = Memory::new("123", "@test");
        assert_eq!(mem.frontmatter.user_id, "123");
        assert_eq!(mem.frontmatter.username, "@test");
        assert!(mem.frontmatter.call_name.is_empty());
        assert!(mem.frontmatter.description.is_empty());
        assert!(mem.body.is_empty());
    }

    #[test]
    fn test_memory_append_log() {
        let mut mem = Memory::new("123", "@test");
        mem.append_log("likes Rust programming.");
        mem.append_log("uses NixOS.");
        assert!(mem.body.contains("likes Rust programming"));
        assert!(mem.body.contains("uses NixOS"));
    }

    #[test]
    fn test_memory_to_file_content() {
        let mut mem = Memory::new("123", "@test");
        mem.frontmatter.call_name = "Tester".into();
        mem.frontmatter.description = "A test user".into();
        mem.append_log("did something cool.");
        let content = mem.to_file_content();
        assert!(content.contains("user_id:"));
        assert!(content.contains("call_name:"));
        // Verify it can be parsed back
        let parsed = parse_memory(&content).unwrap();
        assert_eq!(parsed.frontmatter.user_id, "123");
        assert_eq!(parsed.frontmatter.call_name, "Tester");
    }

    #[test]
    fn test_memory_to_system_prompt() {
        let mut mem = Memory::new("123", "@test");
        let prompt = mem.to_system_prompt();
        assert!(prompt.contains("User ID: 123"));
        assert!(prompt.contains("Username: @test"));
        // call_name and description should not appear when empty
        assert!(!prompt.contains("Call them:"));
        assert!(!prompt.contains("About them:"));

        mem.frontmatter.call_name = "Testy".into();
        mem.frontmatter.description = "Loves testing.".into();
        let prompt = mem.to_system_prompt();
        assert!(prompt.contains("Call them: Testy"));
        assert!(prompt.contains("About them: Loves testing."));
    }

    #[test]
    fn test_save_and_load_memory() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let mut mem = Memory::new("123", "@test");
        mem.frontmatter.call_name = "Tester".into();
        mem.append_log("likes tests.");

        save_memory(&chats_dir, "-456", "123", &mem).unwrap();
        let loaded = load_memory(&chats_dir, "-456", "123").unwrap();
        assert_eq!(loaded.frontmatter.call_name, "Tester");
        assert!(loaded.body.contains("likes tests"));
    }

    #[test]
    fn test_load_memory_nonexistent() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let result = load_memory(&chats_dir, "-456", "nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_memory_invalid() {
        let content = "Not a---valid---memory";
        assert!(parse_memory(content).is_none());
    }

    #[test]
    fn test_parse_memory_minimal_frontmatter() {
        let content = "---\nuser_id: \"123\"\n---\nsome body";
        let mem = parse_memory(content).unwrap();
        assert_eq!(mem.frontmatter.user_id, "123");
        assert_eq!(mem.body, "some body");
    }

    #[test]
    fn test_memory_append_log_first_entry() {
        let mut mem = Memory::new("123", "@test");
        mem.append_log("first entry.");
        assert!(mem.body.contains("first entry"));
        assert!(!mem.body.starts_with('\n'));
    }

    #[test]
    fn test_memory_to_file_content_empty_body() {
        let mem = Memory::new("123", "@test");
        let content = mem.to_file_content();
        assert!(content.contains("user_id:"));
        assert!(!content.ends_with('\n'));
    }

    #[test]
    fn test_load_chat_memories_ignores_non_md() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let chat_dir = chats_dir.join("-100");
        std::fs::create_dir_all(&chat_dir).unwrap();
        // Create a non-.md file
        std::fs::write(chat_dir.join("notes.txt"), "not memory").unwrap();
        let loaded = load_chat_memories(&chats_dir, "-100").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_load_chat_memories() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let mut mem1 = Memory::new("111", "@user1");
        mem1.frontmatter.call_name = "User1".into();
        let mut mem2 = Memory::new("222", "@user2");
        mem2.frontmatter.call_name = "User2".into();
        save_memory(&chats_dir, "-100", "111", &mem1).unwrap();
        save_memory(&chats_dir, "-100", "222", &mem2).unwrap();

        let loaded = load_chat_memories(&chats_dir, "-100").unwrap();
        assert_eq!(loaded.len(), 2);
        let names: Vec<_> = loaded
            .iter()
            .map(|m| m.frontmatter.call_name.clone())
            .collect();
        assert!(names.contains(&"User1".to_string()));
        assert!(names.contains(&"User2".to_string()));
    }

    #[test]
    fn test_load_chat_memories_empty() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let result = load_chat_memories(&chats_dir, "-none").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_chat_memory_new() {
        let mem = Memory::new_chat();
        assert_eq!(mem.frontmatter.user_id, "_chat");
        assert!(mem.frontmatter.call_name.is_empty());
        assert!(mem.frontmatter.description.is_empty());
        assert!(mem.body.is_empty());
    }

    #[test]
    fn test_save_and_load_chat_memory() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let mut mem = Memory::new_chat();
        mem.frontmatter.call_name = "Study Group".into();
        mem.frontmatter.description = "A group for learning Rust".into();
        mem.append_log("started the chat");

        save_chat_memory(&chats_dir, "-100", &mem).unwrap();
        let loaded = load_chat_memory(&chats_dir, "-100").unwrap();
        assert_eq!(loaded.frontmatter.call_name, "Study Group");
        assert_eq!(loaded.frontmatter.description, "A group for learning Rust");
        assert!(loaded.body.contains("started the chat"));
    }

    #[test]
    fn test_load_chat_memory_nonexistent() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        assert!(load_chat_memory(&chats_dir, "-none").is_none());
    }

    #[test]
    fn test_load_chat_memories_excludes_chat_file() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        // Create a user memory
        save_memory(&chats_dir, "-100", "111", &Memory::new("111", "@u1")).unwrap();
        // Create a chat memory
        save_chat_memory(&chats_dir, "-100", &Memory::new_chat()).unwrap();

        let users = load_chat_memories(&chats_dir, "-100").unwrap();
        assert_eq!(users.len(), 1); // only user memory, not _chat.md
        assert_eq!(users[0].frontmatter.user_id, "111");
    }

    #[test]
    fn test_to_chat_system_prompt() {
        let mut mem = Memory::new_chat();
        // Empty chat memory produces empty prompt
        assert_eq!(mem.to_chat_system_prompt(), "");

        mem.frontmatter.call_name = "My Chat".into();
        mem.frontmatter.description = "Testing stuff".into();
        let prompt = mem.to_chat_system_prompt();
        assert!(prompt.contains("My Chat"));
        assert!(prompt.contains("Testing stuff"));
    }

    #[test]
    fn test_to_system_prompt_with_body() {
        let mut mem = Memory::new("123", "@test");
        mem.append_log("likes Rust.");
        mem.append_log("uses NixOS.");
        let prompt = mem.to_system_prompt();
        assert!(prompt.contains("Memory log:"));
        assert!(prompt.contains("likes Rust"));
        assert!(prompt.contains("uses NixOS"));
    }

    #[test]
    fn test_to_chat_system_prompt_with_body() {
        let mut mem = Memory::new_chat();
        mem.append_log("group topic decided.");
        let prompt = mem.to_chat_system_prompt();
        assert!(prompt.contains("History:"));
        assert!(prompt.contains("group topic decided"));
    }

    #[test]
    fn test_to_chat_system_prompt_only_name() {
        let mut mem = Memory::new_chat();
        mem.frontmatter.call_name = "Just Name".into();
        let prompt = mem.to_chat_system_prompt();
        assert!(prompt.contains("Just Name"));
        assert!(!prompt.contains("About:"));
    }

    #[test]
    fn test_chat_memory_append_log() {
        let mut mem = Memory::new_chat();
        mem.append_log("group topic decided");
        mem.append_log("new member joined");
        assert!(mem.body.contains("group topic decided"));
        assert!(mem.body.contains("new member joined"));
    }
}
