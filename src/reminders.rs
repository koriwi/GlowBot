use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single reminder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: String,
    /// What to remind the user about — sent as the message when the reminder fires
    /// (if action is None) or used as context for the action.
    pub description: String,
    /// Optional: what the LLM should do when the reminder fires.
    /// For example: "Look up mom's phone number from memory and put it in the chat."
    /// If set, the heartbeat agent processes this as a one-off task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// ISO 8601 timestamp when this reminder should fire (e.g. "2026-05-11T18:00:00Z").
    pub trigger_at: String,
    /// Timestamp when the reminder was created.
    pub created_at: String,
}

/// A chat's reminder list.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReminderList {
    #[serde(default)]
    pub reminders: Vec<Reminder>,
}

impl ReminderList {
    /// Load reminders from a chat's reminders.yaml file.
    pub fn load(chats_dir: &Path, chat_id: &str) -> anyhow::Result<Self> {
        let path = chats_dir.join(chat_id).join("reminders.yaml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(&path)?;
        let list: Self = serde_yaml::from_str(&data).unwrap_or_default();
        Ok(list)
    }

    /// Save reminders to a chat's reminders.yaml file.
    pub fn save(&self, chats_dir: &Path, chat_id: &str) -> anyhow::Result<()> {
        let dir = chats_dir.join(chat_id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("reminders.yaml");
        let data = serde_yaml::to_string(self)?;
        std::fs::write(&path, data)?;
        Ok(())
    }

    /// Add a reminder and return its ID.
    pub fn add(
        &mut self,
        description: &str,
        trigger_at: &str,
        action: Option<&str>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.reminders.push(Reminder {
            id: id.clone(),
            description: description.to_string(),
            action: action.map(|a| a.to_string()),
            trigger_at: trigger_at.to_string(),
            created_at: chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
        });
        id
    }

    /// Remove a reminder by ID.
    pub fn remove(&mut self, id: &str) -> bool {
        let len_before = self.reminders.len();
        self.reminders.retain(|r| r.id != id);
        self.reminders.len() < len_before
    }

    /// Get all reminders whose trigger_at is in the past.
    pub fn due(&self) -> Vec<&Reminder> {
        let now = chrono::Utc::now();
        self.reminders
            .iter()
            .filter(|r| {
                chrono::DateTime::parse_from_rfc3339(&r.trigger_at)
                    .map(|dt| dt < now)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Check if there are any reminders.
    pub fn has_reminders(&self) -> bool {
        !self.reminders.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_add_and_remove_reminder() {
        let mut list = ReminderList::default();
        let id = list.add("Call mom", "2026-05-11T18:00:00Z", None);
        assert_eq!(list.reminders.len(), 1);
        assert!(list.has_reminders());

        let r = &list.reminders[0];
        assert_eq!(r.description, "Call mom");
        assert_eq!(r.trigger_at, "2026-05-11T18:00:00Z");
        assert!(r.action.is_none());

        assert!(list.remove(&id));
        assert!(!list.has_reminders());
        assert!(!list.remove("nonexistent"));
    }

    #[test]
    fn test_add_reminder_with_action() {
        let mut list = ReminderList::default();
        let id = list.add(
            "Check stock price",
            "2026-05-11T09:00:00Z",
            Some("Look up AAPL stock price via curl and report it"),
        );
        let r = list.reminders.iter().find(|r| r.id == id).unwrap();
        assert_eq!(
            r.action.as_deref(),
            Some("Look up AAPL stock price via curl and report it")
        );
    }

    #[test]
    fn test_due_reminders() {
        let mut list = ReminderList::default();
        // Past reminder — should be due
        list.add(
            "Past reminder",
            "2020-01-01T00:00:00Z",
            None,
        );
        // Future reminder — not due
        list.add(
            "Future reminder",
            "2099-12-31T23:59:59Z",
            None,
        );
        let due = list.due();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].description, "Past reminder");
    }

    #[test]
    fn test_due_with_invalid_timestamp() {
        let mut list = ReminderList::default();
        list.add("Invalid", "not-a-timestamp", None);
        let due = list.due();
        assert!(due.is_empty());
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");

        let mut list = ReminderList::default();
        list.add("Reminder one", "2026-06-01T12:00:00Z", None);
        list.add(
            "Reminder two",
            "2026-06-02T12:00:00Z",
            Some("Do something"),
        );
        list.save(&chats_dir, "-123").unwrap();

        let loaded = ReminderList::load(&chats_dir, "-123").unwrap();
        assert_eq!(loaded.reminders.len(), 2);
        assert_eq!(loaded.reminders[0].description, "Reminder one");
        assert_eq!(loaded.reminders[1].action.as_deref(), Some("Do something"));
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("chats");
        let list = ReminderList::load(&chats_dir, "-none").unwrap();
        assert!(list.reminders.is_empty());
    }
}
