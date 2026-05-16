use anyhow::Context;
use rusqlite::Connection;
use std::path::Path;

/// Build a temporary reference database from the schema `.sql` files,
/// run `sqldiff --schema` to compute the delta, and apply it to the
/// live database.
pub(crate) fn migrate_with_sqldiff(db_path: &Path, schema_dir: &Path) -> anyhow::Result<()> {
    // Build a temporary reference database with the desired schema.
    let ref_file = tempfile::NamedTempFile::new()
        .context("Failed to create temp file for reference database")?;
    let ref_conn =
        Connection::open(ref_file.path()).context("Failed to open reference database")?;

    for entry in std::fs::read_dir(schema_dir)
        .with_context(|| format!("Failed to read schema dir: {}", schema_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sql") {
            let sql = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read schema file: {}", path.display()))?;
            ref_conn
                .execute_batch(&sql)
                .with_context(|| format!("Failed to execute schema file: {}", path.display()))?;
        }
    }
    drop(ref_conn);

    // Diff the live database against the reference.
    let output = std::process::Command::new("sqldiff")
        .args([
            "--schema",
            db_path.to_str().context("db_path is not valid UTF-8")?,
            ref_file
                .path()
                .to_str()
                .context("ref_path is not valid UTF-8")?,
        ])
        .output()
        .context("Failed to run sqldiff. Is it installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("sqldiff failed:\n{}", stderr);
    }

    let diff_sql = String::from_utf8_lossy(&output.stdout);
    let diff_sql = diff_sql.trim();
    if diff_sql.is_empty() {
        log::info!("Database schema is up to date.");
        return Ok(());
    }

    log::info!("Applying schema migration:\n{}", diff_sql);

    // Apply the diff to the live database (open a fresh connection).
    let conn = Connection::open(db_path).with_context(|| {
        format!(
            "Failed to open database for migration: {}",
            db_path.display()
        )
    })?;
    conn.execute_batch(diff_sql)
        .context("Failed to apply migration SQL")?;

    Ok(())
}

/// Direct schema initialisation — used for tests and as a fallback
/// when `sqldiff` is not available.
pub(crate) fn init_direct(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            chat_id     TEXT    NOT NULL,
            role        TEXT    NOT NULL,
            content     TEXT    NOT NULL,
            name        TEXT,
            tool_calls  TEXT,
            tool_call_id TEXT,
            created_at  INTEGER NOT NULL,
            reasoning   TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_messages_chat_created
         ON messages(chat_id, created_at);
        CREATE TABLE IF NOT EXISTS message_embeddings (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            embedding   BLOB NOT NULL,
            model       TEXT NOT NULL,
            created_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_embeddings_message
         ON message_embeddings(message_id);
        CREATE INDEX IF NOT EXISTS idx_embeddings_model_message
         ON message_embeddings(model, message_id);
        CREATE TABLE IF NOT EXISTS chat_cutoffs (
            chat_id  TEXT    PRIMARY KEY,
            cutoff_at INTEGER NOT NULL
        );",
    )
    .context("Failed to initialize database schema")?;

    // Migration: add reasoning column if it doesn't exist (for databases
    // created before this column was added).
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN reasoning TEXT", []);

    Ok(())
}
