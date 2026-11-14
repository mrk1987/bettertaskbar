//! Per-widget double-click choices. Resolving a choice has no side effects.
use crate::{app::Kind, config::Config};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    OpenUrl,
    Refresh,
    LaunchApp,
}
impl Action {
    pub const ALL: [Self; 3] = [Self::OpenUrl, Self::Refresh, Self::LaunchApp];
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenUrl => "Open URL",
            Self::Refresh => "Refresh widget",
            Self::LaunchApp => "Launch app",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Missing in old files: URLs for weather/calendar/mail, refresh for usage.
    pub action: Option<Action>,
    /// Empty uses the widget's default URL, following the weather location.
    pub url: String,
    pub app: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Url(String),
    Refresh,
    App(PathBuf),
}

impl Settings {
    pub fn action(&self, kind: Kind) -> Action {
        self.action.unwrap_or(if matches!(kind, Kind::Usage(_)) {
            Action::Refresh
        } else {
            Action::OpenUrl
        })
    }

    pub fn resolve(&self, kind: Kind, cfg: &Config) -> Result<Target, String> {
        match self.action(kind) {
            Action::Refresh => Ok(Target::Refresh),
            Action::OpenUrl => {
                let text = if self.url.trim().is_empty() {
                    kind.default_url(cfg)
                } else {
                    self.url.trim().into()
                };
                if text.chars().any(char::is_control) {
                    return Err("Enter a web URL without line breaks or control characters.".into());
                }
                let parsed = url::Url::parse(&text)
                    .map_err(|_| "Enter a complete http:// or https:// URL.")?;
                if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                    return Err("Enter a complete http:// or https:// URL.".into());
                }
                Ok(Target::Url(parsed.into()))
            }
            Action::LaunchApp => {
                let text = self.app.trim();
                if text.is_empty() || text.chars().any(char::is_control) {
                    return Err("Choose an application (.exe) to launch.".into());
                }
                let path = PathBuf::from(text);
                if !path.is_absolute()
                    || !path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                {
                    return Err(
                        "Choose a full path to an application (.exe), without quotes or arguments."
                            .into(),
                    );
                }
                Ok(Target::App(path))
            }
        }
    }

    /// Merge individual edits so changing a URL cannot undo a menu action
    /// choice, a concurrent drag, or an edit to another widget.
    pub fn merge_into(&self, base: &Self, latest: &mut Self) {
        if self.action != base.action {
            latest.action = self.action;
        }
        if self.url != base.url {
            latest.url = self.url.clone();
        }
        if self.app != base.app {
            latest.app = self.app.clone();
        }
    }

    pub fn refresh_from(&mut self, base: &mut Self, fresh: &Self) {
        if self.action == base.action {
            self.action = fresh.action;
        }
        if self.url == base.url {
            self.url = fresh.url.clone();
        }
        if self.app == base.app {
            self.app = fresh.app.clone();
        }
        *base = fresh.clone();
    }
}

pub fn launch(path: &std::path::Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(
            "The selected app was not found. Choose its current .exe file in settings.".into(),
        );
    }
    let mut command = std::process::Command::new(path);
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }
    // This is an executable path, never a shell command or an argument string.
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Cannot launch the selected app: {e}"))
}

pub fn choose_app() -> Option<String> {
    use windows::{
        core::{w, PCWSTR, PWSTR},
        Win32::UI::Controls::Dialogs::*,
    };
    let mut buffer = [0u16; 4096];
    let filter: Vec<u16> = "Applications (*.exe)\0*.exe\0\0".encode_utf16().collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: crate::shell::find_by_title(crate::dashboard::TITLE).unwrap_or_default(),
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrDefExt: w!("exe"),
        Flags: OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut dialog) }.as_bool() {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0)?;
    Some(String::from_utf16_lossy(&buffer[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_menu_switches_do_not_overwrite_unsaved_destinations() {
        let mut base = Settings::default();
        let mut draft = base.clone();
        draft.url = "https://example.com/inbox".into();
        let mut latest = Settings {
            action: Some(Action::Refresh),
            app: "C:\\Apps\\Mail.exe".into(),
            ..Default::default()
        };
        draft.refresh_from(&mut base, &latest);
        assert_eq!(draft.action, Some(Action::Refresh));
        assert_eq!(draft.url, "https://example.com/inbox");
        draft.merge_into(&base, &mut latest);
        assert_eq!(latest.action, Some(Action::Refresh));
        assert_eq!(latest.url, draft.url);
        assert_eq!(latest.app, "C:\\Apps\\Mail.exe");
        let serialized = serde_json::to_string(&latest).unwrap();
        assert_eq!(
            serde_json::from_str::<Settings>(&serialized).unwrap(),
            latest
        );
        draft.refresh_from(&mut base, &latest);
        assert_eq!(draft, base);
    }
    #[test]
    fn legacy_defaults_and_weather_url_follow_the_current_place() {
        let mut cfg = Config::default();
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.action(Kind::Mail), Action::OpenUrl);
        assert_eq!(
            settings.action(Kind::Usage(
                codezeno_usage_core::providers::ProviderId::Codex
            )),
            Action::Refresh
        );
        cfg.weather.place.as_mut().unwrap().lat = 10.0;
        assert!(
            matches!(settings.resolve(Kind::Weather, &cfg).unwrap(), Target::Url(u) if u.contains("10.0000"))
        );
    }
    #[test]
    fn url_and_app_targets_cannot_be_commands_or_other_protocols() {
        let cfg = Config::default();
        let mut s = Settings {
            action: Some(Action::OpenUrl),
            ..Default::default()
        };
        for text in [
            "file:///C:/test.exe",
            "javascript:alert(1)",
            "https://example.com/\nrun",
            "not a URL",
            "ms-settings:",
        ] {
            s.url = text.into();
            assert!(s.resolve(Kind::Mail, &cfg).is_err(), "{text}");
        }
        s.url = "https://example.com/a?x=1&y=2".into();
        assert!(matches!(s.resolve(Kind::Mail, &cfg), Ok(Target::Url(_))));
        s.action = Some(Action::LaunchApp);
        for text in [
            "cmd /c whoami",
            "app.exe",
            "C:\\apps\\run.cmd",
            "\"C:\\apps\\app.exe\" --arg",
            "C:\\apps\\app.exe\0",
        ] {
            s.app = text.into();
            assert!(s.resolve(Kind::Mail, &cfg).is_err(), "{text}");
        }
        s.app = "C:\\Program Files\\Example & Co\\app.exe".into();
        assert_eq!(
            s.resolve(Kind::Mail, &cfg).unwrap(),
            Target::App(PathBuf::from(&s.app))
        );
    }
}
