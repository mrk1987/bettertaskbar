use crate::theme::Theme;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use windows::{
    core::{w, PCWSTR, PWSTR},
    Win32::UI::Controls::Dialogs::*,
};
pub fn choose(save: bool) -> Option<PathBuf> {
    let mut buffer = [0u16; 4096];
    let filter: Vec<u16> = "Widget themes (*.json)\0*.json\0\0"
        .encode_utf16()
        .collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: crate::shell::find_by_title(crate::dashboard::TITLE).unwrap_or_default(),
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrDefExt: w!("json"),
        Flags: OFN_NOCHANGEDIR
            | OFN_PATHMUSTEXIST
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            },
        ..Default::default()
    };
    let ok = unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    };
    if !ok.as_bool() {
        return None;
    }
    let length = buffer.iter().position(|c| *c == 0)?;
    use std::os::windows::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length],
    )))
}
pub fn read(path: &Path) -> Result<Theme, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("Theme files must be smaller than 256 KB.".into());
    }
    let theme: Theme =
        serde_json::from_slice(&bytes).map_err(|e| format!("Cannot read theme: {e}"))?;
    theme.validate()?;
    Ok(theme)
}
pub fn write(path: &Path, theme: &Theme) -> Result<(), String> {
    theme.validate()?;
    let bytes = serde_json::to_vec_pretty(theme).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
        .map_err(|e| e.to_string())
}
