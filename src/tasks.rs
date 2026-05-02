use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single task in the task list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub created_at: String,
}

/// A chat's task list.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskList {
    #[serde(default)]
    pub tasks: Vec<Task>,
}

impl TaskList {
    /// Load tasks from a chat's tasks.yaml file.
    pub fn load(chats_dir: &Path, chat_id: &str) -> anyhow::Result<Self> {
        let path = chats_dir.join(chat_id).join("tasks.yaml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(&path)?;
        let list: Self = serde_yaml::from_str(&data).unwrap_or_default();
        Ok(list)
    }

    /// Save tasks to a chat's tasks.yaml file.
    pub fn save(&self, chats_dir: &Path, chat_id: &str) -> anyhow::Result<()> {
        let dir = chats_dir.join(chat_id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("tasks.yaml");
        let data = serde_yaml::to_string(self)?;
        std::fs::write(&path, data)?;
        Ok(())
    }

    /// Add a task and return its ID.
    pub fn add(&mut self, description: &str) -> String {
        let id = format!("{}", self.tasks.len() + 1);
        self.tasks.push(Task {
            id: id.clone(),
            description: description.to_string(),
            created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        });
        id
    }

    /// Remove a task by ID.
    pub fn remove(&mut self, id: &str) -> bool {
        let len_before = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        self.tasks.len() < len_before
    }

    /// Get the oldest task (first in the list).
    pub fn oldest(&self) -> Option<&Task> {
        self.tasks.first()
    }

    /// Check if there are any tasks.
    pub fn has_tasks(&self) -> bool {
        !self.tasks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_add_and_remove_task() {
        let mut list = TaskList::default();
        let id = list.add("Test task");
        assert_eq!(list.tasks.len(), 1);
        assert!(list.has_tasks());
        assert!(list.oldest().is_some());

        assert!(list.remove(&id));
        assert!(!list.has_tasks());
        assert!(!list.remove("nonexistent"));
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");

        let mut list = TaskList::default();
        list.add("Task one");
        list.add("Task two");
        list.save(&chats_dir, "-123").unwrap();

        let loaded = TaskList::load(&chats_dir, "-123").unwrap();
        assert_eq!(loaded.tasks.len(), 2);
        assert_eq!(loaded.tasks[0].description, "Task one");
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let list = TaskList::load(&chats_dir, "-none").unwrap();
        assert!(list.tasks.is_empty());
    }
}
