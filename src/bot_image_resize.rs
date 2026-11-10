use anyhow::Context;
use std::path::Path;
use tokio::process::Command;

pub(super) fn reference_limit(args: &serde_json::Value) -> anyhow::Result<Option<u64>> {
    match args.get("reference_max_dimension") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => match value.as_u64() {
            Some(n @ 1..=16384) => Ok(Some(n)),
            _ => anyhow::bail!("reference_max_dimension must be an integer from 1 to 16384"),
        },
    }
}

pub(super) async fn read_reference(path: &Path, limit: Option<u64>) -> anyhow::Result<Vec<u8>> {
    let bytes = tokio::fs::read(path)
        .await
        .context("Cannot read reference")?;
    let Some(limit) = limit else { return Ok(bytes) };
    // A controlled temporary filename avoids ImageMagick interpreting input paths
    // as options, image selectors, or format prefixes. Original files are untouched.
    let input = tempfile::NamedTempFile::new()?;
    tokio::fs::write(input.path(), &bytes).await?;
    let mut command = Command::new("convert");
    command.kill_on_drop(true).args([
        &format!("{}[0]", input.path().display()),
        "-auto-orient",
        "-resize",
        &format!("{limit}x{limit}>"),
        "-strip",
        "png:-",
    ]);
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
        .await
        .context("Reference image resize timed out")?
        .context("Cannot run ImageMagick convert; install imagemagick")?;
    if !output.status.success() {
        anyhow::bail!(
            "ImageMagick resize failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    anyhow::ensure!(
        !output.stdout.is_empty(),
        "ImageMagick returned an empty image"
    );
    Ok(output.stdout)
}

#[cfg(test)]
#[path = "bot_image_resize_tests.rs"]
mod tests;
