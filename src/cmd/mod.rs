pub mod add;
pub mod annotate;
pub mod close;
pub mod edit;
pub mod ls;
pub mod open;
pub mod pending;
pub mod report;
pub mod setup;
pub mod tick;

use crate::db::KINDS;
use anyhow::{Result, ensure};

pub fn check_kind(k: &str) -> Result<()> {
    ensure!(KINDS.contains(&k), "kind must be one of {}, got {k:?}", KINDS.join("|"));
    Ok(())
}

pub fn icon(kind: &str) -> &'static str {
    match kind {
        "todo" => "▶",
        "discuss" => "❓",
        "closeable" => "✅",
        _ => "•",
    }
}

/// Last path component, used as the display name of a configured project.
pub fn project_label(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

pub fn none_if_empty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.trim().is_empty())
}
