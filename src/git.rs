use anyhow::Context;
use std::path::Path;

/// Run git commands in the data directory.
#[derive(Clone)]
pub struct GitRepo {
    repo_path: std::path::PathBuf,
}

impl GitRepo {
    /// Create a new GitRepo handler for the given path.
    /// Also marks the directory as safe for git to avoid "dubious ownership" errors
    /// when running inside Docker containers.
    pub fn new(repo_path: &Path) -> Self {
        // Mark the data directory as safe for git (important in Docker where
        // the volume may be owned by a different user).
        let _ = std::process::Command::new("git")
            .args(["config", "--global", "--add", "safe.directory"])
            .arg(repo_path)
            .output();
        // Set git user identity for commits.
        let _ = std::process::Command::new("git")
            .args(["config", "--global", "user.email", "glowy@glowythebot.com"])
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "--global", "user.name", "GlowBot"])
            .output();
        Self {
            repo_path: repo_path.to_path_buf(),
        }
    }

    /// Check if the data directory is a git repository.
    pub fn is_repo(&self) -> bool {
        self.repo_path.join(".git").exists()
    }

    /// Initialize a git repository if not already one.
    pub fn init(&self) -> anyhow::Result<()> {
        if self.is_repo() {
            return Ok(());
        }
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to init git repo")?;
        Ok(())
    }

    /// Stage all changes, commit, and push.
    pub fn auto_commit(&self, message: &str) -> anyhow::Result<()> {
        if !self.is_repo() {
            return Ok(());
        }
        // git add -A
        let status = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to git add")?;
        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            anyhow::bail!("git add failed: {}", stderr);
        }

        // Check if there is anything to commit
        let diff = std::process::Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(&self.repo_path)
            .status()
            .context("Failed to check git diff")?;
        if diff.success() {
            // Nothing to commit
            return Ok(());
        }

        // git commit
        let status = std::process::Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to git commit")?;
        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            anyhow::bail!("git commit failed: {}", stderr);
        }

        // git push (ignore errors if no remote)
        let _ = std::process::Command::new("git")
            .args(["push"])
            .current_dir(&self.repo_path)
            .output();

        Ok(())
    }

    /// Get list of files changed since last commit (or all untracked/modified).
    pub fn changed_files(&self) -> anyhow::Result<Vec<String>> {
        let output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to get git status")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<String> = stdout
            .lines()
            .map(|line| line[3..].trim().to_string())
            .collect();
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_new_repo() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        assert!(!repo.is_repo());
        repo.init().unwrap();
        assert!(repo.is_repo());
    }

    #[test]
    fn test_init_existing_repo() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        repo.init().unwrap();
        assert!(repo.is_repo());
        // init again should not error
        repo.init().unwrap();
    }

    #[test]
    fn test_auto_commit_no_changes() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        repo.init().unwrap();
        // Should not error when there's nothing to commit
        let result = repo.auto_commit("test commit");
        assert!(result.is_ok());
    }

    #[test]
    fn test_auto_commit_with_changes() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        repo.init().unwrap();

        // Create a file
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();

        let result = repo.auto_commit("add test file");
        assert!(result.is_ok());

        // Verify commit exists
        let output = std::process::Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(log.contains("add test file"));
    }

    #[test]
    fn test_changed_files() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        repo.init().unwrap();

        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();

        let files = repo.changed_files().unwrap();
        assert!(files.contains(&"a.txt".to_string()));
        assert!(files.contains(&"b.txt".to_string()));
    }

    #[test]
    fn test_no_push_on_no_remote() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        repo.init().unwrap();
        std::fs::write(dir.path().join("f.txt"), "content").unwrap();
        // Should not panic even without remote
        let result = repo.auto_commit("test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_auto_commit_not_a_repo() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        // Should not error, just skip
        let result = repo.auto_commit("test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_is_repo_false() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        assert!(!repo.is_repo());
    }

    #[test]
    fn test_changed_files_empty() {
        let dir = TempDir::new().unwrap();
        let repo = GitRepo::new(dir.path());
        repo.init().unwrap();
        let files = repo.changed_files().unwrap();
        assert!(files.is_empty());
    }
}
