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
    tools_enabled: bool,
    bash_enabled: bool,
    _user_id: &str,
    media_dir: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    // 1. Base prompt (with chat context)
    parts.push(base_prompt(chat_id, media_dir, tools_enabled, bash_enabled));

    // 2. Per-chat system prompt
    if !chat_system_prompt.is_empty() {
        parts.push(format!(
            "\n## Chat-specific instructions\n{}",
            chat_system_prompt
        ));
    }

    // 3. Skills
    if !skills.is_empty() {
        parts.push(
            "\n## Available skills\n\nYou have the following skills available. Each skill contains bash commands, workflows, or other instructions. Use the `read_skill` tool to load the full content of a skill when you need it. Do not include the full body here.\n".to_string(),
        );
        for skill in skills.values() {
            parts.push(format!(
                "- **{}** – {}",
                skill.frontmatter.name, skill.frontmatter.description
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

    // 6. DM tool restriction notice
    if !tools_enabled {
        parts.push(format!(
            "\n## Important: Tool Access Restricted\n\
You are in a DM without a `dms` config entry. All your tools (bash, memory, skills, tasks, \
media, conversation search) are currently DISABLED. You can only respond with text.\n\
If the user asks you to do something that requires tools, tell them:\n\
> To enable tools, ask the bot owner to add a `dms` entry for this chat ID (`{chat_id}`) \
in `config.yaml` and restart me.\n\
Always include the chat ID (`{chat_id}`) in that message.",
            chat_id = chat_id
        ));
    }

    parts.join("\n")
}

/// The base system prompt that all messages start with.
fn base_prompt(chat_id: &str, media_dir: &str, tools_enabled: bool, bash_enabled: bool) -> String {
    let tools_intro = if tools_enabled && bash_enabled {
        "You have access to tools for executing bash commands, managing memory, creating skills, handling tasks, sending media, and more."
    } else if tools_enabled {
        "You have access to tools for managing memory, creating skills, handling tasks, sending media, and more. The bash tool is disabled in this chat."
    } else {
        "Your tools are currently restricted."
    };

    let mut prompt = format!(
        r#"You are GlowBot, a helpful, friendly Telegram chatbot. {tools_intro}

Your personality:
- Be concise and helpful.
- Address users by their call_name when you know it.
- You may use **bold** and *italic* formatting in your replies (Telegram Markdown).
- Use `backticks` for code, commands, file paths, and technical terms.
- Telegram does not support Markdown tables. Wrap tables in triple backticks (```) as code blocks so they render as preformatted text.
- When you learn something worth remembering about a user, use update_memory to save it.
- When you learn something about the chat/group itself (topics, purpose, participants, dynamics), use read_chat_memory to recall and update_chat_memory to save it.
- User and chat memories are included in the system prompt above with their logged facts. You can still use the read_* tools to check raw files if needed.
- Always use the memory tools (read_memory, update_memory, read_chat_memory, update_chat_memory) to access memory files — never raw bash. The structured tools guarantee correct YAML frontmatter format.
- You can create and update skills with the create_skill and update_skill tools. Skills are Markdown files that extend your capabilities with bash commands or workflows. Only create or update skills when explicitly asked by a user — do not create skills proactively.
- Recent conversation history is included as separate messages alongside this prompt (up to the configured window size). If older messages were trimmed or you need more context, call `get_recent_messages(count)` to retrieve them from the database.
- For semantic search across past conversations (long-term memory), use `search_conversations(query, count?)` — describe the topic or question you're looking for (e.g. "what did Alice say about the deadline?"). Returns ranked results with similarity scores.
- When you expect to make several tool calls before answering, or a task will take a moment, use `send_message` to give the user a quick headsup (e.g. "ok, give me a second, taking a look now..."). Err on the side of sending it — users appreciate knowing you're working. At most once per turn, and never for your final answer (which is sent automatically).
- In background tasks (heartbeat), use `send_message` once at the end to report completion or deliver results when the user explicitly asked. Do not spam progress updates.
- The current chat ID is: {chat_id}
- Use `send_media` to send files to the chat — it accepts absolute paths, relative paths (from the data directory), or paths inside the media directory at `{media_dir}`. Use `list_media` to browse available media files before sending.
- When using the Playwright browser automation tool (MCP), it saves all screenshots, downloads, and generated files to `{media_dir}/pw-media`. This is its root/working directory — all file paths returned by Playwright are relative to `{media_dir}/pw-media`.
- You can manage a per-chat task list with `add_task`, `list_tasks`, and `remove_task`. The bot autonomously works on tasks on a heartbeat timer.
"#,
        tools_intro = tools_intro,
        chat_id = chat_id,
        media_dir = media_dir,
    );

    if tools_enabled && bash_enabled {
        prompt.push_str(&format!(
            "- Use bash for files, APIs, and system tasks. Use curl, jq, grep, find, and other standard Unix tools.\n- Never run destructive commands (rm -rf, format, etc.) unless explicitly asked.\n- If a command fails, try to diagnose and fix it.\n"
        ));
    }

    prompt.push_str(&format!(
        "\nCurrent date: {}",
        chrono::Utc::now().format("%Y-%m-%d")
    ));

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_basic() {
        let prompt = assemble(
            "-123",
            "",
            &HashMap::new(),
            None,
            &[],
            true,
            true,
            "456",
            "/media",
        );
        assert!(prompt.contains("GlowBot"));
        assert!(prompt.contains("bash"));
        assert!(prompt.contains("-123"));
        assert!(prompt.contains("pw-media"));
        assert!(prompt.contains("/media/pw-media"));
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        assert!(prompt.contains(&today));
    }

    #[test]
    fn test_assemble_bash_disabled() {
        let prompt = assemble(
            "-123",
            "",
            &HashMap::new(),
            None,
            &[],
            true,
            false,
            "456",
            "/media",
        );
        assert!(prompt.contains("GlowBot"));
        assert!(prompt.contains("bash tool is disabled"));
        assert!(!prompt.contains("Never run destructive commands"));
        assert!(!prompt.contains("curl, jq, grep"));
    }

    #[test]
    fn test_assemble_tools_disabled_intro() {
        let prompt = assemble(
            "-123",
            "",
            &HashMap::new(),
            None,
            &[],
            false,
            false,
            "456",
            "/media",
        );
        assert!(prompt.contains("tools are currently restricted"));
        // DM restriction mentions "bash" in its disabled-tools list,
        // but the intro should not mention bash as available.
        assert!(!prompt.contains("executing bash commands"));
        assert!(prompt.contains("Tool Access Restricted"));
    }

    #[test]
    fn test_assemble_with_chat_prompt() {
        let prompt = assemble(
            "-123",
            "Be extra helpful in this chat.",
            &HashMap::new(),
            None,
            &[],
            true,
            true,
            "456",
            "/media",
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
        let prompt = assemble("-123", "", &skills, None, &[], true, true, "456", "/media");
        assert!(prompt.contains("test-skill"));
        assert!(prompt.contains("A test skill"));
        // Skill body "Use curl to do things." must NOT be in the prompt —
        // only the name and description should appear.
        assert!(!prompt.contains("Use curl to do things"));
        assert!(prompt.contains("read_skill"));
    }

    #[test]
    fn test_assemble_with_memories() {
        let mut mem = Memory::new("123", "@testuser");
        mem.frontmatter.call_name = "Tester".into();
        mem.frontmatter.description = "Loves testing.".into();
        mem.append_log("wrote 50 tests today.");
        let prompt = assemble(
            "-123",
            "",
            &HashMap::new(),
            None,
            &[mem],
            true,
            true,
            "456",
            "/media",
        );
        assert!(prompt.contains("Tester"));
        assert!(prompt.contains("@testuser"));
        assert!(prompt.contains("Known users"));
        assert!(prompt.contains("Memory log:"));
        assert!(prompt.contains("wrote 50 tests today"));
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
        let prompt = assemble(
            "-123",
            "chat prompt here",
            &skills,
            None,
            &[mem],
            true,
            true,
            "456",
            "/media",
        );
        assert!(prompt.contains("GlowBot"));
        assert!(prompt.contains("chat prompt here"));
        assert!(prompt.contains("s1"));
    }

    #[test]
    fn test_assemble_with_chat_memory() {
        let mut chat_mem = Memory::new_chat();
        chat_mem.frontmatter.call_name = "Study Group".into();
        chat_mem.frontmatter.description = "We learn Rust together".into();
        chat_mem.append_log("started learning enums.");
        let prompt = assemble(
            "-123",
            "",
            &HashMap::new(),
            Some(&chat_mem),
            &[],
            true,
            true,
            "456",
            "/media",
        );
        assert!(prompt.contains("About this conversation"));
        assert!(prompt.contains("Study Group"));
        assert!(prompt.contains("History:"));
        assert!(prompt.contains("started learning enums"));
    }

    #[test]
    fn test_assemble_with_empty_chat_memory() {
        let chat_mem = Memory::new_chat();
        let prompt = assemble(
            "-123",
            "",
            &HashMap::new(),
            Some(&chat_mem),
            &[],
            true,
            true,
            "456",
            "/media",
        );
        assert!(!prompt.contains("About this conversation"));
    }

    #[test]
    fn test_assemble_dm_tools_disabled() {
        let prompt = assemble(
            "123",
            "",
            &HashMap::new(),
            None,
            &[],
            false,
            false,
            "789",
            "/media",
        );
        assert!(prompt.contains("Tool Access Restricted"));
        assert!(prompt.contains("123"));
        assert!(prompt.contains("DISABLED"));
    }

    #[test]
    fn test_assemble_dm_tools_enabled() {
        let prompt = assemble(
            "123",
            "",
            &HashMap::new(),
            None,
            &[],
            true,
            true,
            "789",
            "/media",
        );
        assert!(!prompt.contains("Tool Access Restricted"));
    }

    #[test]
    fn test_assemble_uses_custom_media_dir() {
        let prompt = assemble(
            "123",
            "",
            &HashMap::new(),
            None,
            &[],
            true,
            true,
            "789",
            "/custom_media",
        );
        assert!(prompt.contains("/custom_media"));
        assert!(prompt.contains("/custom_media/pw-media"));
    }

    #[test]
    fn test_assemble_no_bash_instructions() {
        let prompt = assemble(
            "-123",
            "",
            &HashMap::new(),
            None,
            &[],
            true,
            false,
            "456",
            "/media",
        );
        assert!(prompt.contains("memory tools"));
        assert!(prompt.contains("list_media"));
        assert!(prompt.contains("add_task"));
        assert!(!prompt.contains("curl, jq, grep"));
        assert!(!prompt.contains("Never run destructive"));
    }
}
