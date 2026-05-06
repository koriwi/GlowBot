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
