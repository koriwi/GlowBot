use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Frontmatter for a skill markdown file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
}

/// A parsed skill loaded from disk.
#[derive(Debug, Clone)]
pub struct Skill {
    /// The frontmatter metadata.
    pub frontmatter: SkillFrontmatter,
    /// The full raw content of the skill (including frontmatter + body).
    pub raw: String,
    /// The body text (everything after the frontmatter).
    pub body: String,
}

/// Parse YAML frontmatter from a markdown string.
/// Expects content starting with `---\n` and ending with `---\n`.
pub fn parse_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;
    let end = content.find("\n---")?;
    let frontmatter = &content[..end];
    let body_start = end + 4; // skip "\n---"
    let body = &content[body_start..];
    let body = body.strip_prefix('\n').unwrap_or(body);
    Some((frontmatter, body))
}

/// Load a single skill from a skill.md file.
pub fn load_skill(path: &Path) -> anyhow::Result<Skill> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read skill file: {}", path.display()))?;
    parse_skill_from_string(&content)
        .with_context(|| format!("Failed to parse skill file: {}", path.display()))
}

/// Parse a skill from its string content.
pub fn parse_skill_from_string(content: &str) -> anyhow::Result<Skill> {
    let (frontmatter_str, body) =
        parse_frontmatter(content).context("Skill must have YAML frontmatter delimited by ---")?;
    let frontmatter: SkillFrontmatter =
        serde_yaml::from_str(frontmatter_str).context("Failed to parse skill frontmatter YAML")?;
    Ok(Skill {
        frontmatter,
        raw: content.to_string(),
        body: body.to_string(),
    })
}

/// Load all skills from a skills directory.
/// Recursively looks for `skill.md` files in subdirectories.
pub fn load_all_skills(skills_dir: &Path) -> anyhow::Result<HashMap<String, Skill>> {
    let mut skills = HashMap::new();
    if !skills_dir.exists() {
        return Ok(skills);
    }
    let entries = std::fs::read_dir(skills_dir)
        .with_context(|| format!("Failed to read skills directory: {}", skills_dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let skill_file = path.join("skill.md");
            if skill_file.exists() {
                let skill = load_skill(&skill_file)?;
                skills.insert(skill.frontmatter.name.clone(), skill);
            }
        }
    }
    Ok(skills)
}

/// Write a skill file to disk.
pub fn write_skill(
    skills_dir: &Path,
    name: &str,
    frontmatter: &SkillFrontmatter,
    body: &str,
) -> anyhow::Result<PathBuf> {
    let dir = skills_dir.join(name);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create skill directory: {}", dir.display()))?;
    let yaml = serde_yaml::to_string(frontmatter)?;
    let content = format!("---\n{}---\n{}", yaml, body);
    let path = dir.join("skill.md");
    std::fs::write(&path, &content)
        .with_context(|| format!("Failed to write skill file: {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = "---\nname: test-skill\ndescription: A test skill\n---\nThis is the body.\n";
        let (fm, body) = parse_frontmatter(content).unwrap();
        assert_eq!(fm, "name: test-skill\ndescription: A test skill");
        assert_eq!(body, "This is the body.\n");
    }

    #[test]
    fn test_parse_frontmatter_empty_body() {
        let content = "---\nname: test\ndescription: desc\n---\n";
        let (_fm, body) = parse_frontmatter(content).unwrap();
        assert_eq!(body, "");
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "No frontmatter here.";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_unclosed() {
        let content = "---\nname: test\n";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_skill_from_string() {
        let content =
            "---\nname: search-web\ndescription: Searches the web\n---\nUse curl to search.\n";
        let skill = parse_skill_from_string(content).unwrap();
        assert_eq!(skill.frontmatter.name, "search-web");
        assert_eq!(skill.frontmatter.description, "Searches the web");
        assert_eq!(skill.body, "Use curl to search.\n");
        assert_eq!(skill.raw, content);
    }

    #[test]
    fn test_parse_skill_missing_fields() {
        let content = "---\nname: incomplete\n---\nbody\n";
        let skill = parse_skill_from_string(content);
        // Should fail because description is missing
        assert!(skill.is_err());
    }

    #[test]
    fn test_load_and_write_skill() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        let fm = SkillFrontmatter {
            name: "my-skill".into(),
            description: "Does things".into(),
        };
        let path = write_skill(&skills_dir, "my-skill", &fm, "The body text.\n").unwrap();
        assert!(path.exists());

        let loaded = load_skill(&path).unwrap();
        assert_eq!(loaded.frontmatter.name, "my-skill");
        assert_eq!(loaded.frontmatter.description, "Does things");
        assert_eq!(loaded.body, "The body text.\n");
    }

    #[test]
    fn test_load_all_skills_empty() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();
        let skills = load_all_skills(&skills_dir).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_load_all_skills_nonexistent() {
        let skills = load_all_skills(Path::new("/nonexistent/skills")).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_load_all_skills_multiple() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");

        let fm1 = SkillFrontmatter {
            name: "skill-a".into(),
            description: "First skill".into(),
        };
        write_skill(&skills_dir, "skill-a", &fm1, "body a\n").unwrap();

        let fm2 = SkillFrontmatter {
            name: "skill-b".into(),
            description: "Second skill".into(),
        };
        write_skill(&skills_dir, "skill-b", &fm2, "body b\n").unwrap();

        let skills = load_all_skills(&skills_dir).unwrap();
        assert_eq!(skills.len(), 2);
        assert!(skills.contains_key("skill-a"));
        assert!(skills.contains_key("skill-b"));
    }

    #[test]
    fn test_load_all_skills_ignores_non_dirs() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        // Create a file directly in skills_dir (not a directory)
        std::fs::write(skills_dir.join("not_a_skill.md"), "nope").unwrap();
        let skills = load_all_skills(&skills_dir).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_load_skill_nonexistent_file() {
        let result = load_skill(Path::new("/nonexistent/skill.md"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_skill_invalid_frontmatter() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("badskill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("skill.md"), "not valid yaml frontmatter").unwrap();
        let result = load_skill(&skill_dir.join("skill.md"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_frontmatter_crlf() {
        let content = "---\r\nname: test\r\ndescription: desc\r\n---\r\nbody here";
        let (fm, body) = parse_frontmatter(content).unwrap();
        assert!(fm.contains("name: test"));
        assert_eq!(body.trim(), "body here");
    }
}
