use crate::memory::Memory;
use crate::skills::Skill;
use std::collections::HashMap;

/// Assemble the full system prompt for a given context.
pub fn assemble(
    chat_id: &str,
    chat_system_prompt: &str,
    skills: &HashMap<String, Skill>,
    chat_memory: Option<&Memory>,
    memories: &[Memory],
) -> String {
    let mut parts: Vec<String> = Vec::new();

    // 1. Base prompt (with chat context)
    parts.push(base_prompt(chat_id));

    // 2. Per-chat system prompt
    if !chat_system_prompt.is_empty() {
        parts.push(format!(
            "\n## Chat-specific instructions\n{}",
            chat_system_prompt
        ));
    }

    // 3. Skills
    if !skills.is_empty() {
        parts.push("\n## Available skills\n".to_string());
        for skill in skills.values() {
            parts.push(format!(
                "### {}\n{}\n\n{}",
                skill.frontmatter.name, skill.frontmatter.description, skill.body
            ));
        }
    }

    // 4. Chat-level memory
    if let Some(chat_mem) = chat_memory {
        let prompt = chat_mem.to_chat_system_prompt();
        if !prompt.is_empty() {
            parts.push(format!("\n## About this conversation\n{}", prompt));
        }
    }

    // 5. User memories
    if !memories.is_empty() {
        parts.push("\n## Known users in this conversation\n".to_string());
        for memory in memories {
            parts.push(memory.to_system_prompt());
        }
    }

    parts.join("\n")
}

/// The base system prompt that all messages start with.
fn base_prompt(chat_id: &str) -> String {
    format!(
        r#"You are GlowBot, a helpful, friendly Telegram chatbot. You have access to tools for executing bash commands and managing memory.

Your personality:
- Be concise and helpful.
- Address users by their call_name when you know it.
- You may use **bold** and *italic* formatting in your replies (Telegram Markdown).
- Use `backticks` for code, commands, file paths, and technical terms.
- Telegram does not support Markdown tables. Wrap tables in triple backticks (```) as code blocks so they render as preformatted text.
- Use tools freely — bash for files/APIs/skills, memory tools for user and chat context.
- Before answering a question whose answer depends on the user (timezone, preferences, name, skill level, location, OS, etc.), ALWAYS call read_memory first to check what you know about them. Never guess — look it up.
- When you learn something worth remembering about a user, use update_memory to save it.
- When you learn something about the chat/group itself (topics, purpose, participants, dynamics), use update_chat_memory to save it.
- You already have user memories in the system prompt above — but they only show frontmatter summaries. Call read_memory to see the full body with logged facts.
- You can create and update skills with the create_skill and update_skill tools. Skills are Markdown files that extend your capabilities with bash commands or workflows. When a user asks you to build a new capability, create a skill for it.
- The current chat ID is: {chat_id}
- Memory files live under chats/{chat_id}/ — you can also read them raw with bash if needed.
- You may use curl, jq, grep, find, and other standard Unix tools via bash.
- Never run destructive commands (rm -rf, format, etc.) unless explicitly asked.
- If a command fails, try to diagnose and fix it.

Current date: "#,
        chat_id = chat_id,
    ) + &chrono::Utc::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_basic() {
        let prompt = assemble("-123", "", &HashMap::new(), None, &[]);
        assert!(prompt.contains("GlowBot"));
        assert!(prompt.contains("bash"));
        // Should contain the chat ID
        assert!(prompt.contains("-123"));
        // Should contain the date
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        assert!(prompt.contains(&today));
    }

    #[test]
    fn test_assemble_with_chat_prompt() {
        let prompt = assemble(
            "-123",
            "Be extra helpful in this chat.",
            &HashMap::new(),
            None,
            &[],
        );
        assert!(prompt.contains("Be extra helpful"));
        assert!(prompt.contains("Chat-specific instructions"));
    }

    #[test]
    fn test_assemble_with_skills() {
        use crate::skills::SkillFrontmatter;
        let mut skills = HashMap::new();
        skills.insert(
            "test-skill".into(),
            Skill {
                frontmatter: SkillFrontmatter {
                    name: "test-skill".into(),
                    description: "A test skill".into(),
                },
                raw: String::new(),
                body: "Use curl to do things.\n".into(),
            },
        );
        let prompt = assemble("-123", "", &skills, None, &[]);
        assert!(prompt.contains("test-skill"));
        assert!(prompt.contains("A test skill"));
        assert!(prompt.contains("Use curl"));
        assert!(prompt.contains("Available skills"));
    }

    #[test]
    fn test_assemble_with_memories() {
        let mut mem = Memory::new("123", "@testuser");
        mem.frontmatter.call_name = "Tester".into();
        mem.frontmatter.description = "Loves testing.".into();
        let prompt = assemble("-123", "", &HashMap::new(), None, &[mem]);
        assert!(prompt.contains("Tester"));
        assert!(prompt.contains("@testuser"));
        assert!(prompt.contains("Loves testing."));
        assert!(prompt.contains("Known users"));
    }

    #[test]
    fn test_assemble_with_everything() {
        use crate::skills::SkillFrontmatter;
        let mut skills = HashMap::new();
        skills.insert(
            "s1".into(),
            Skill {
                frontmatter: SkillFrontmatter {
                    name: "s1".into(),
                    description: "desc1".into(),
                },
                raw: String::new(),
                body: "body1\n".into(),
            },
        );
        let mut mem = Memory::new("111", "@u1");
        mem.frontmatter.call_name = "U1".into();
        let prompt = assemble("-123", "chat prompt here", &skills, None, &[mem]);
        assert!(prompt.contains("GlowBot"));
        assert!(prompt.contains("chat prompt here"));
        assert!(prompt.contains("s1"));
        assert!(prompt.contains("U1"));
    }

    #[test]
    fn test_assemble_with_chat_memory() {
        let mut chat_mem = Memory::new_chat();
        chat_mem.frontmatter.call_name = "Study Group".into();
        chat_mem.frontmatter.description = "We learn Rust together".into();

        let prompt = assemble("-123", "", &HashMap::new(), Some(&chat_mem), &[]);
        assert!(prompt.contains("About this conversation"));
        assert!(prompt.contains("Study Group"));
        assert!(prompt.contains("We learn Rust together"));
    }

    #[test]
    fn test_assemble_with_empty_chat_memory() {
        let chat_mem = Memory::new_chat();
        let prompt = assemble("-123", "", &HashMap::new(), Some(&chat_mem), &[]);
        // Empty chat memory should not produce a section header
        assert!(!prompt.contains("About this conversation"));
    }
}
