use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single human todo item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub completed: bool,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// A chat's todo list.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TodoList {
    #[serde(default)]
    pub todos: Vec<Todo>,
}

impl TodoList {
    /// Load todos from a chat's todos.yaml file.
    pub fn load(chats_dir: &Path, chat_id: &str) -> anyhow::Result<Self> {
        let path = chats_dir.join(chat_id).join("todos.yaml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(&path)?;
        let list: Self = serde_yaml::from_str(&data).unwrap_or_default();
        Ok(list)
    }

    /// Save todos to a chat's todos.yaml file.
    pub fn save(&self, chats_dir: &Path, chat_id: &str) -> anyhow::Result<()> {
        let dir = chats_dir.join(chat_id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("todos.yaml");
        let data = serde_yaml::to_string(self)?;
        std::fs::write(&path, data)?;
        Ok(())
    }

    /// Add a todo and return its ID.
    pub fn add(&mut self, description: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.todos.push(Todo {
            id: id.clone(),
            description: description.to_string(),
            completed: false,
            created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            updated_at: None,
        });
        id
    }

    /// Remove a todo by ID.
    pub fn remove(&mut self, id: &str) -> bool {
        let len_before = self.todos.len();
        self.todos.retain(|t| t.id != id);
        self.todos.len() < len_before
    }

    /// Edit a todo's description by ID. Returns false if not found.
    pub fn edit(&mut self, id: &str, description: &str) -> bool {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        if let Some(t) = self.todos.iter_mut().find(|t| t.id == id) {
            t.description = description.to_string();
            t.updated_at = Some(now);
            true
        } else {
            false
        }
    }

    /// Toggle a todo's completed status by ID. Returns false if not found.
    /// Returns `Some(new_status)` on success.
    pub fn toggle(&mut self, id: &str) -> Option<bool> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        self.todos.iter_mut().find(|t| t.id == id).map(|t| {
            t.completed = !t.completed;
            t.updated_at = Some(now);
            t.completed
        })
    }

    /// Check if there are any todos.
    pub fn has_todos(&self) -> bool {
        !self.todos.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_add_and_remove_todo() {
        let mut list = TodoList::default();
        let id = list.add("Buy groceries");
        assert_eq!(list.todos.len(), 1);
        assert!(list.has_todos());
        assert!(!list.todos[0].completed);

        assert!(list.remove(&id));
        assert!(!list.has_todos());
        assert!(!list.remove("nonexistent"));
    }

    #[test]
    fn test_edit_todo() {
        let mut list = TodoList::default();
        let id = list.add("Original desc");
        assert!(list.edit(&id, "Updated desc"));
        assert_eq!(list.todos[0].description, "Updated desc");
        assert!(list.todos[0].updated_at.is_some());
        assert!(!list.edit("nonexistent", "nope"));
    }

    #[test]
    fn test_toggle_todo() {
        let mut list = TodoList::default();
        let id = list.add("Test toggle");
        assert!(!list.todos[0].completed);

        let result = list.toggle(&id);
        assert_eq!(result, Some(true));
        assert!(list.todos[0].completed);
        assert!(list.todos[0].updated_at.is_some());

        let result = list.toggle(&id);
        assert_eq!(result, Some(false));
        assert!(!list.todos[0].completed);

        assert_eq!(list.toggle("nonexistent"), None);
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");

        let mut list = TodoList::default();
        list.add("Todo one");
        let id2 = list.add("Todo two");
        list.toggle(&id2); // mark as completed
        list.save(&chats_dir, "-123").unwrap();

        let loaded = TodoList::load(&chats_dir, "-123").unwrap();
        assert_eq!(loaded.todos.len(), 2);
        assert_eq!(loaded.todos[0].description, "Todo one");
        assert!(!loaded.todos[0].completed);
        assert_eq!(loaded.todos[1].description, "Todo two");
        assert!(loaded.todos[1].completed);
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let list = TodoList::load(&chats_dir, "-none").unwrap();
        assert!(list.todos.is_empty());
    }

    #[test]
    fn test_edit_updates_updated_at() {
        let mut list = TodoList::default();
        let id = list.add("A todo");
        assert!(list.todos[0].updated_at.is_none());

        list.edit(&id, "Changed");
        assert!(list.todos[0].updated_at.is_some());

        list.toggle(&id);
        // updated_at should be refreshed after toggle too
        assert!(list.todos[0].updated_at.is_some());
    }
}
