use crate::config::CodexConfig;
use anyhow::Context;
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

const OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const AUTH_CLAIM: &str = "https://api.openai.com/auth";

pub(crate) async fn access_token(
    config: &CodexConfig,
    client: &reqwest::Client,
    auth_lock: &Mutex<()>,
) -> anyhow::Result<String> {
    access_token_with_url(config, client, auth_lock, TOKEN_URL).await
}

pub(crate) async fn access_token_with_url(
    config: &CodexConfig,
    client: &reqwest::Client,
    auth_lock: &Mutex<()>,
    token_url: &str,
) -> anyhow::Result<String> {
    let _guard = auth_lock.lock().await;
    let path = expand_home(&config.auth_file)?;
    let mut auth = read_auth(&path)?;
    let format = auth_format(&auth)?;
    let access = stored_token(&auth, format, "access")
        .context("Codex auth file has no access token; run `codex login` or Pi's `/login`")?;

    if token_valid_for(access, 300)? {
        return Ok(access.to_string());
    }

    let refresh = stored_token(&auth, format, "refresh")
        .context("Codex access token expired and auth file has no refresh token")?
        .to_string();
    let refreshed = refresh_token(client, &refresh, token_url).await?;
    persist_refreshed_tokens(&mut auth, format, &refreshed)?;
    write_auth(&path, &auth)?;
    Ok(refreshed.access_token)
}

#[derive(Clone, Copy)]
enum AuthFormat {
    CodexCli,
    Pi,
}

fn auth_format(auth: &Value) -> anyhow::Result<AuthFormat> {
    if auth.get("tokens").and_then(Value::as_object).is_some() {
        Ok(AuthFormat::CodexCli)
    } else if auth
        .get("openai-codex")
        .and_then(Value::as_object)
        .is_some()
    {
        Ok(AuthFormat::Pi)
    } else {
        anyhow::bail!(
            "Unsupported Codex auth file format; use ~/.codex/auth.json or Pi's ~/.pi/agent/auth.json"
        )
    }
}

fn stored_token<'a>(auth: &'a Value, format: AuthFormat, token: &str) -> Option<&'a str> {
    let (container, key) = match (format, token) {
        (AuthFormat::CodexCli, "access") => ("tokens", "access_token"),
        (AuthFormat::CodexCli, "refresh") => ("tokens", "refresh_token"),
        (AuthFormat::Pi, "access") => ("openai-codex", "access"),
        (AuthFormat::Pi, "refresh") => ("openai-codex", "refresh"),
        _ => return None,
    };
    auth.get(container)?.get(key)?.as_str()
}

fn persist_refreshed_tokens(
    auth: &mut Value,
    format: AuthFormat,
    refreshed: &RefreshResponse,
) -> anyhow::Result<()> {
    let (container, access_key, refresh_key) = match format {
        AuthFormat::CodexCli => ("tokens", "access_token", "refresh_token"),
        AuthFormat::Pi => ("openai-codex", "access", "refresh"),
    };
    let tokens = auth
        .get_mut(container)
        .and_then(Value::as_object_mut)
        .context("Codex auth file has an invalid credential object")?;
    tokens.insert(
        access_key.into(),
        Value::String(refreshed.access_token.clone()),
    );
    if let Some(refresh_token) = &refreshed.refresh_token {
        tokens.insert(refresh_key.into(), Value::String(refresh_token.clone()));
    }
    if let Some(id_token) = &refreshed.id_token {
        tokens.insert("id_token".into(), Value::String(id_token.clone()));
    }
    match format {
        AuthFormat::CodexCli => {
            auth["last_refresh"] = Value::String(chrono::Utc::now().to_rfc3339());
        }
        AuthFormat::Pi => {
            tokens.insert(
                "expires".into(),
                Value::from(token_expiry_millis(&refreshed.access_token)?),
            );
        }
    }
    Ok(())
}

fn token_expiry_millis(token: &str) -> anyhow::Result<i64> {
    Ok(jwt_payload(token)?
        .get("exp")
        .and_then(Value::as_i64)
        .context("Codex access token has no expiry")?
        * 1000)
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

async fn refresh_token(
    client: &reqwest::Client,
    refresh: &str,
    token_url: &str,
) -> anyhow::Result<RefreshResponse> {
    let response = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", OAUTH_CLIENT_ID),
        ])
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "Codex OAuth refresh failed ({status}): {}. Run `codex login` again",
            truncate(&body, 1000)
        );
    }
    serde_json::from_str(&body).context("Invalid Codex OAuth refresh response")
}

pub(crate) fn expand_home(path: &str) -> anyhow::Result<PathBuf> {
    if path == "~" || path.starts_with("~/") {
        let home =
            std::env::var_os("HOME").context("HOME is not set; use an absolute codex.auth_file")?;
        return Ok(PathBuf::from(home).join(path.trim_start_matches("~/")));
    }
    Ok(PathBuf::from(path))
}

fn read_auth(path: &Path) -> anyhow::Result<Value> {
    let data = std::fs::read_to_string(path).with_context(|| {
        format!(
            "Failed to read Codex auth file {}. Run `codex login` first",
            path.display()
        )
    })?;
    serde_json::from_str(&data).context("Failed to parse Codex auth file")
}

fn write_auth(path: &Path, auth: &Value) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("Codex auth file has no parent directory")?;
    let temp = parent.join(format!(".auth.json.glowbot-{}", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(auth)?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&bytes)?;
    }
    #[cfg(not(unix))]
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

fn jwt_payload(token: &str) -> anyhow::Result<Value> {
    let payload = token
        .split('.')
        .nth(1)
        .context("Codex access token is not a JWT")?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("Invalid Codex access token encoding")?;
    serde_json::from_slice(&decoded).context("Invalid Codex access token payload")
}

pub(crate) fn account_id(token: &str) -> anyhow::Result<String> {
    jwt_payload(token)?
        .get(AUTH_CLAIM)
        .and_then(|claim| claim.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .context("Codex access token has no ChatGPT account ID")
}

pub(crate) fn token_valid_for(token: &str, seconds: i64) -> anyhow::Result<bool> {
    let expiry = jwt_payload(token)?
        .get("exp")
        .and_then(Value::as_i64)
        .context("Codex access token has no expiry")?;
    Ok(expiry > chrono::Utc::now().timestamp() + seconds)
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.into()
    } else {
        format!("{}...", value.chars().take(max).collect::<String>())
    }
}
