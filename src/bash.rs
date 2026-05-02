use anyhow::Context;
use tokio::process::Command;

/// Maximum execution time for a bash command.
const BASH_TIMEOUT_SECS: u64 = 30;

/// Result of a bash command execution.
#[derive(Debug, Clone)]
pub struct BashResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Execute a oneshot bash command. Stateless, non-interactive.
pub async fn execute(command: &str) -> anyhow::Result<BashResult> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(BASH_TIMEOUT_SECS),
        Command::new("bash").args(["-c", command]).output(),
    )
    .await
    .context("Bash command timed out")?
    .context("Failed to execute bash command")?;

    Ok(BashResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

/// Execute a bash command in a specific working directory.
pub async fn execute_in_dir(command: &str, dir: &std::path::Path) -> anyhow::Result<BashResult> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(BASH_TIMEOUT_SECS),
        Command::new("bash")
            .args(["-c", command])
            .current_dir(dir)
            .output(),
    )
    .await
    .context("Bash command timed out")?
    .context("Failed to execute bash command")?;

    Ok(BashResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute_simple_echo() {
        let result = execute("echo hello").await.unwrap();
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.exit_code, 0);
        assert!(result.stderr.is_empty());
    }

    #[tokio::test]
    async fn test_execute_with_stderr() {
        let result = execute("echo error >&2").await.unwrap();
        assert!(result.stderr.contains("error"));
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_execute_exit_code() {
        let result = execute("exit 42").await.unwrap();
        assert_eq!(result.exit_code, 42);
    }

    #[tokio::test]
    async fn test_execute_empty_command() {
        let result = execute("").await.unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_execute_in_dir() {
        let dir = std::env::temp_dir();
        let result = execute_in_dir("pwd", &dir).await.unwrap();
        assert!(result
            .stdout
            .trim()
            .contains(&dir.to_string_lossy().to_string()));
    }

    #[tokio::test]
    async fn test_execute_multiline() {
        let result = execute("echo line1 && echo line2").await.unwrap();
        let lines: Vec<&str> = result.stdout.trim().lines().collect();
        assert_eq!(lines, vec!["line1", "line2"]);
    }
}
