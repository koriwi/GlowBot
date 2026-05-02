use crate::memory::Memory;
use crate::skills::Skill;
use std::collections::HashMap;

/// Assemble the full system prompt for a given context.
pub fn assemble(
    chat_id: &str,
    chat_system_prompt: &str,
    skills: &HashMap<String, Skill>,
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

    // 4. User memories
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
        r#"You are GlowBot, a helpful, friendly Telegram chatbot. You have access to tools for executing bash commands and managing user memory.

Your personality:
- Be concise and helpful.
- Address users by their call_name when you know it.
- Use tools freely — bash for files/APIs/skills, read_memory and update_memory for user context.
- When you learn something worth remembering about a user, use update_memory to save it.
- Use read_memory at the start of a conversation to recall what you know about the user.
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
        let prompt = assemble("-123", "", &HashMap::new(), &[]);
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
        let prompt = assemble("-123", "", &skills, &[]);
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
        let prompt = assemble("-123", "", &HashMap::new(), &[mem]);
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
        let prompt = assemble("-123", "chat prompt here", &skills, &[mem]);
        assert!(prompt.contains("GlowBot"));
        assert!(prompt.contains("chat prompt here"));
        assert!(prompt.contains("s1"));
        assert!(prompt.contains("U1"));
    }
}
