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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::BotState;
    use crate::llm::mock::MockLlmBackend;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    async fn make_state() -> (Arc<Mutex<BotState>>, TempDir) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let config = crate::config::basic_config();
        config.save(&data_dir.join("config.yaml")).unwrap();
        let mock_llm: Arc<dyn crate::llm::LlmBackend> = Arc::new(MockLlmBackend::new());
        let state = Arc::new(Mutex::new(BotState {
            config,
            skills: std::collections::HashMap::new(),
            llm: mock_llm,
            data_dir: data_dir.clone(),
            db: crate::db::Database::open_in_memory().unwrap(),
            mcp_tools: vec![],
            _mcp_services: vec![],
            mcp_peers: std::collections::HashMap::new(),
            model_metadata: std::collections::HashMap::new(),
            model_order: vec![],
            last_usage: std::collections::HashMap::new(),
            pending_config_changes: std::collections::HashMap::new(),
            pending_model_changes: std::collections::HashMap::new(),
            model_overrides: std::collections::HashMap::new(),
            last_browse_cb: std::collections::HashMap::new(),
        }));
        (state, dir)
    }

    // ─── create_skill ────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_skill_success() {
        let (state, _dir) = make_state().await;
        let args = json!({
            "name": "my-skill",
            "description": "A test skill",
            "body": "This is the skill body"
        });
        let result = tool_create_skill(&state, &args).await;
        assert!(result.contains("created"));

        // Verify the skill was written
        let path = state
            .lock()
            .await
            .skills_dir()
            .join("my-skill")
            .join("skill.md");
        let skill = crate::skills::load_skill(&path).unwrap();
        assert_eq!(skill.frontmatter.name, "my-skill");
        assert_eq!(skill.body, "This is the skill body");
    }

    #[tokio::test]
    async fn test_create_skill_missing_name() {
        let (state, _dir) = make_state().await;
        let args = json!({"description": "desc", "body": "body"});
        let result = tool_create_skill(&state, &args).await;
        assert!(result.contains("Error"));
        assert!(result.contains("required"));
    }

    #[tokio::test]
    async fn test_create_skill_missing_body() {
        let (state, _dir) = make_state().await;
        let args = json!({"name": "sk", "description": "desc"});
        let result = tool_create_skill(&state, &args).await;
        assert!(result.contains("Error"));
        assert!(result.contains("required"));
    }

    // ─── read_skill ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_read_skill_not_found() {
        let (state, _dir) = make_state().await;
        let args = json!({"name": "nonexistent"});
        let result = tool_read_skill(&state, &args).await;
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_read_skill_exists() {
        let (state, _dir) = make_state().await;
        let fm = crate::skills::SkillFrontmatter {
            name: "test-skill".into(),
            description: "A test".into(),
        };
        crate::skills::write_skill(
            &state.lock().await.skills_dir(),
            "test-skill",
            &fm,
            "skill body content",
        )
        .unwrap();

        let args = json!({"name": "test-skill"});
        let result = tool_read_skill(&state, &args).await;
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["name"], "test-skill");
        assert_eq!(v["body"], "skill body content");
    }

    #[tokio::test]
    async fn test_read_skill_empty_name() {
        let (state, _dir) = make_state().await;
        let args = json!({"name": ""});
        let result = tool_read_skill(&state, &args).await;
        assert!(result.contains("Error"));
        assert!(result.contains("required"));
    }

    // ─── update_skill ────────────────────────────────────────────

    #[tokio::test]
    async fn test_update_skill_not_found() {
        let (state, _dir) = make_state().await;
        let args = json!({"name": "nonexistent", "body": "new body"});
        let result = tool_update_skill(&state, &args).await;
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_update_skill_success() {
        let (state, _dir) = make_state().await;
        let fm = crate::skills::SkillFrontmatter {
            name: "update-me".into(),
            description: "Original desc".into(),
        };
        crate::skills::write_skill(
            &state.lock().await.skills_dir(),
            "update-me",
            &fm,
            "original body",
        )
        .unwrap();

        let args =
            json!({"name": "update-me", "description": "Updated desc", "body": "updated body"});
        let result = tool_update_skill(&state, &args).await;
        assert!(result.contains("updated"));

        let path = state
            .lock()
            .await
            .skills_dir()
            .join("update-me")
            .join("skill.md");
        let skill = crate::skills::load_skill(&path).unwrap();
        assert_eq!(skill.frontmatter.description, "Updated desc");
        assert_eq!(skill.body, "updated body");
    }

    #[tokio::test]
    async fn test_update_skill_no_changes() {
        let (state, _dir) = make_state().await;
        let fm = crate::skills::SkillFrontmatter {
            name: "unchanged".into(),
            description: "Desc".into(),
        };
        crate::skills::write_skill(&state.lock().await.skills_dir(), "unchanged", &fm, "body")
            .unwrap();

        let args = json!({"name": "unchanged"});
        let result = tool_update_skill(&state, &args).await;
        assert_eq!(result, "No fields to update.");
    }

    #[tokio::test]
    async fn test_update_skill_empty_name() {
        let (state, _dir) = make_state().await;
        let args = json!({"name": ""});
        let result = tool_update_skill(&state, &args).await;
        assert!(result.contains("Error"));
        assert!(result.contains("required"));
    }
}
