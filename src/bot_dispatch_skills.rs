use super::BotState;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle the `create_skill` tool — write a new skill directory with skill.md.
pub(crate) async fn tool_create_skill(
    state: &Arc<Mutex<BotState>>,
    args: &serde_json::Value,
) -> String {
    let name = args["name"].as_str().unwrap_or("");
    let desc = args["description"].as_str().unwrap_or("");
    let body = args["body"].as_str().unwrap_or("");
    if name.is_empty() || desc.is_empty() || body.is_empty() {
        return "Error: name, description, body required".into();
    }
    let s = state.lock().await;
    let fm = crate::skills::SkillFrontmatter {
        name: name.into(),
        description: desc.into(),
    };
    match crate::skills::write_skill(&s.skills_dir(), name, &fm, body) {
        Ok(_) => format!("Skill '{}' created", name),
        Err(e) => format!("Error: {}", e),
    }
}

/// Handle the `read_skill` tool — load a skill's skill.md file.
pub(crate) async fn tool_read_skill(
    state: &Arc<Mutex<BotState>>,
    args: &serde_json::Value,
) -> String {
    let name = args["name"].as_str().unwrap_or("");
    if name.is_empty() {
        return "Error: name required".into();
    }
    let s = state.lock().await;
    let path = s.skills_dir().join(name).join("skill.md");
    match crate::skills::load_skill(&path) {
        Ok(skill) => serde_json::json!({
            "name": skill.frontmatter.name,
            "description": skill.frontmatter.description,
            "body": skill.body,
        })
        .to_string(),
        Err(_) => format!("Skill '{}' not found", name),
    }
}

/// Handle the `update_skill` tool — update a skill's description and/or body.
pub(crate) async fn tool_update_skill(
    state: &Arc<Mutex<BotState>>,
    args: &serde_json::Value,
) -> String {
    let name = args["name"].as_str().unwrap_or("");
    if name.is_empty() {
        return "Error: name required".into();
    }
    let s = state.lock().await;
    let path = s.skills_dir().join(name).join("skill.md");
    let mut skill = match crate::skills::load_skill(&path) {
        Ok(s) => s,
        Err(_) => return format!("Skill '{}' not found", name),
    };
    let mut changed = false;
    if let Some(v) = args["description"].as_str() {
        skill.frontmatter.description = v.into();
        changed = true;
    }
    if let Some(v) = args["body"].as_str() {
        skill.body = v.into();
        changed = true;
    }
    if !changed {
        return "No fields to update.".into();
    }
    let yaml = serde_yaml::to_string(&skill.frontmatter).unwrap_or_default();
    let content = format!("---\n{}---\n{}", yaml, skill.body);
    match std::fs::write(&path, &content) {
        Ok(()) => format!("Skill '{}' updated", name),
        Err(e) => format!("Error: {}", e),
    }
}
