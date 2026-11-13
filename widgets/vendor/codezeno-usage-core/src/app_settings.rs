use crate::models::CodexCreditsState;
use std::path::{Path, PathBuf};
pub fn app_data_directory() -> PathBuf {
    let path = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("BetterTaskbar/usage-state");
    let _ = std::fs::create_dir_all(&path);
    path
}

pub fn load_codex_credits() -> Option<CodexCreditsState> {
    serde_json::from_slice(&std::fs::read(app_data_directory().join("codex-credits.json")).ok()?)
        .ok()
}
pub fn save_codex_credits(state: &CodexCreditsState) -> Result<(), String> {
    write_json_atomic(&app_data_directory().join("codex-credits.json"), state)
}
pub fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let write = || -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec(value)?).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    };
    write().map_err(|e| e.to_string())
}
