//! Diagnostics. The widgets are GUI-subsystem processes with no console, so
//! besides stderr every line is appended to `%APPDATA%\BetterTaskbar\widgets.log`.

use std::io::Write;

const MAX_BYTES: u64 = 512 * 1024;

pub fn path() -> std::path::PathBuf {
    crate::config::dir().join("widgets.log")
}

pub fn line(tag: &str, msg: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let text = format!("{stamp} [{tag}] {msg}\n");
    eprint!("{text}");
    let p = path();
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = f.write_all(text.as_bytes());
    }
}
