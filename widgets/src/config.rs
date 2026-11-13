//! Config schema and the shared `widgets.json` file.
//!
//! Four processes (hub + three widgets) share one file. Each keeps a copy in
//! memory, so every write goes through [`update`], which re-reads the file
//! first: a process must never write back its own stale copy, or it would undo
//! another process's edit (or the user's, including credentials).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherCfg {
    pub place: Option<Place>,
    pub favorites: Vec<Place>,
    pub refresh_secs: u64,
}

impl Default for WeatherCfg {
    fn default() -> Self {
        Self {
            place: Some(Place {
                name: "Oslo".into(),
                lat: 59.9139,
                lon: 10.7522,
            }),
            favorites: vec![],
            refresh_secs: 1800,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarCfg {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: String,
    pub password: String,
    /// Where discovery starts: a principal, a calendar home or one calendar.
    pub path: String,
    pub refresh_secs: u64,
}

impl Default for CalendarCfg {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8081,
            tls: false,
            username: String::new(),
            password: String::new(),
            path: "/caldav/".into(),
            refresh_secs: 300,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MailCfg {
    pub host: String,
    pub port: u16,
    /// "starttls" | "tls" | "plain"
    pub security: String,
    pub username: String,
    pub password: String,
    pub refresh_secs: u64,
    pub rotate_secs: u64,
}

impl Default for MailCfg {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 1143,
            security: "starttls".into(),
            username: String::new(),
            password: String::new(),
            refresh_secs: 120,
            rotate_secs: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockSide {
    Left,
    Right,
}

/// Where a widget lives: embedded in the taskbar, on the desktop behind app
/// windows, or free-floating always-on-top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    #[default]
    Taskbar,
    Desktop,
    Floating,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetState {
    pub placement: Placement,
    pub dock_side: Option<DockSide>,
    pub manual_bar_offset: Option<i32>,
    pub enabled: bool,
    pub theme: Option<crate::theme::Theme>,
    pub visible: bool,
    /// Screen position when floating or on the desktop.
    pub x: i32,
    pub y: i32,
    /// When docked: physical pixels between the pill and the notification
    /// area, so the pill keeps its place as tray icons come and go. `None`
    /// until first docked.
    pub tray_offset: Option<i32>,
    /// When docked: device name of the monitor whose taskbar holds the pill
    /// (`\\.\DISPLAY2`); `None` or an absent monitor means the primary bar.
    pub monitor: Option<String>,
}

impl Default for WidgetState {
    fn default() -> Self {
        Self {
            placement: Placement::Taskbar,
            dock_side: None,
            manual_bar_offset: None,
            enabled: true,
            theme: None,
            visible: true,
            x: 24,
            y: 24,
            tray_offset: None,
            monitor: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub magnetized: bool,
    /// Run each pill as its own GPU-drawn eframe process, as before 1.0.1,
    /// instead of drawing every pill in the hub's process on the CPU
    /// (`single.rs`). Same as starting the hub with `--process-per-widget`.
    /// Written to the file only when on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub process_per_widget: bool,
    /// Minutes between usage readings, per enabled provider: one of
    /// `USAGE_REFRESH_CHOICES`. Each reading is a request to the provider.
    pub usage_refresh_minutes: u64,
    pub usage_widgets:
        std::collections::BTreeMap<codezeno_usage_core::providers::ProviderId, WidgetState>,
    pub weather: WeatherCfg,
    pub calendar: CalendarCfg,
    pub mail: MailCfg,
    pub weather_widget: WidgetState,
    pub calendar_widget: WidgetState,
    pub mail_widget: WidgetState,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            magnetized: true,
            process_per_widget: false,
            usage_refresh_minutes: USAGE_REFRESH_CHOICES[0],
            usage_widgets: codezeno_usage_core::providers::ProviderId::ALL
                .into_iter()
                .map(|p| {
                    (
                        p,
                        WidgetState {
                            enabled: false,
                            dock_side: Some(DockSide::Right),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            weather: WeatherCfg::default(),
            calendar: CalendarCfg::default(),
            mail: MailCfg::default(),
            weather_widget: WidgetState::default(),
            calendar_widget: WidgetState::default(),
            mail_widget: WidgetState::default(),
        }
    }
}

pub type SharedConfig = Arc<Mutex<Config>>;

/// The usage refresh intervals the dashboard offers, in minutes; the first is
/// the default.
pub const USAGE_REFRESH_CHOICES: [u64; 4] = [5, 10, 15, 30];

impl Config {
    /// Time between usage readings. Values edited by hand are kept within the
    /// offered range, so a typo cannot hammer a provider.
    pub fn usage_refresh(&self) -> std::time::Duration {
        let first = USAGE_REFRESH_CHOICES[0];
        let last = USAGE_REFRESH_CHOICES[USAGE_REFRESH_CHOICES.len() - 1];
        std::time::Duration::from_secs(self.usage_refresh_minutes.clamp(first, last) * 60)
    }
}

pub fn dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("BetterTaskbar")
}

pub fn path() -> PathBuf {
    dir().join("widgets.json")
}

fn disk_lock(path: &Path) -> std::io::Result<crate::process_lock::Lock> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    crate::process_lock::Lock::acquire(&format!("Local\\BetterTaskbarConfig-{:x}", hash.finish()))
}

enum Disk {
    Ok(Box<Config>),
    Missing,
    /// The file exists but cannot be used right now (half-written by another
    /// process, or a typo while the user is editing it). Never overwrite it.
    Unusable(String),
}

fn read(path: &Path) -> Disk {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(cfg) => Disk::Ok(Box::new(cfg)),
            Err(e) => Disk::Unusable(e.to_string()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Disk::Missing,
        Err(e) => Disk::Unusable(e.to_string()),
    }
}

/// Write via a temp file and rename, so a reader never sees half a file.
fn write(p: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = p.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(cfg).map_err(std::io::Error::other)?,
    )
    .inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    std::fs::rename(&tmp, p).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Load at startup. A missing file is created with defaults; an unreadable one
/// is left alone and defaults are used in memory until it parses again.
pub fn shared() -> SharedConfig {
    let path = path();
    let lock = disk_lock(&path);
    if let Err(e) = &lock {
        crate::log::line("config", &e.to_string());
    }
    let cfg = match read(&path) {
        Disk::Ok(cfg) => *cfg,
        Disk::Missing => {
            let cfg = Config::default();
            if let Err(e) = lock
                .as_ref()
                .map_err(|e| std::io::Error::other(e.to_string()))
                .and_then(|_| write(&path, &cfg))
            {
                crate::log::line("config", &format!("cannot create {}: {e}", path.display()));
            }
            cfg
        }
        Disk::Unusable(e) => {
            crate::log::line(
                "config",
                &format!("widgets.json is not usable, running on defaults: {e}"),
            );
            Config::default()
        }
    };
    Arc::new(Mutex::new(cfg))
}

/// Replace the in-memory copy with what is on disk, if the file parses.
pub fn refresh(shared: &SharedConfig) {
    let path = path();
    let _lock = match disk_lock(&path) {
        Ok(lock) => lock,
        Err(e) => {
            crate::log::line("config", &e.to_string());
            return;
        }
    };
    let mut current = shared.lock().unwrap();
    match read(&path) {
        Disk::Ok(cfg) => *current = *cfg,
        Disk::Missing => {}
        Disk::Unusable(e) => {
            crate::log::line("config", &format!("ignoring unusable widgets.json: {e}"))
        }
    }
}

/// Change the config: re-read the file, apply `f`, write it back. If the file
/// is unusable the change is rejected.
pub fn update(shared: &SharedConfig, f: impl FnOnce(&mut Config)) {
    update_at(shared, &path(), f);
}

fn update_at(shared: &SharedConfig, path: &Path, f: impl FnOnce(&mut Config)) {
    if let Err(error) = try_update_at(shared, path, f) {
        crate::log::line("config", &error);
    }
}

/// Persist one edit against the latest file; report failure to settings UIs.
pub fn try_update(shared: &SharedConfig, f: impl FnOnce(&mut Config)) -> Result<(), String> {
    try_update_checked(shared, |c| {
        f(c);
        Ok(())
    })
}

pub fn try_update_checked(
    shared: &SharedConfig,
    f: impl FnOnce(&mut Config) -> Result<(), String>,
) -> Result<(), String> {
    checked_update_at(shared, &path(), f)
}

fn try_update_at(
    shared: &SharedConfig,
    path: &Path,
    f: impl FnOnce(&mut Config),
) -> Result<(), String> {
    checked_update_at(shared, path, |c| {
        f(c);
        Ok(())
    })
}

fn checked_update_at(
    shared: &SharedConfig,
    path: &Path,
    f: impl FnOnce(&mut Config) -> Result<(), String>,
) -> Result<(), String> {
    let _lock = disk_lock(path).map_err(|e| e.to_string())?;
    let mut current = shared.lock().unwrap();
    let mut candidate = match read(path) {
        Disk::Ok(cfg) => *cfg,
        Disk::Missing => current.clone(),
        Disk::Unusable(error) => {
            return Err(format!("Cannot save: widgets.json is unusable: {error}"))
        }
    };
    f(&mut candidate)?;
    write(path, &candidate).map_err(|e| format!("Cannot save widgets.json: {e}"))?;
    *current = candidate;
    Ok(())
}

/// Last-write time of the config file, for change polling across processes.
pub fn mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(path()).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_refresh_defaults_to_five_minutes_and_stays_in_range() {
        let mut c = Config::default();
        assert_eq!(c.usage_refresh().as_secs(), 300);
        for (minutes, secs) in [(0, 300), (15, 900), (30, 1800), (600, 1800)] {
            c.usage_refresh_minutes = minutes;
            assert_eq!(c.usage_refresh().as_secs(), secs, "{minutes}");
        }
        // Older files without the field get the default.
        let old: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(old.usage_refresh_minutes, 5);
    }

    #[test]
    fn update_worker() {
        let Some(path) = std::env::var_os("BTW_CONFIG_TEST_PATH") else {
            return;
        };
        let cfg = Arc::new(Mutex::new(Config::default()));
        for _ in 0..25 {
            update_at(&cfg, Path::new(&path), |c| c.weather_widget.x += 1);
        }
    }

    #[test]
    fn concurrent_process_updates_preserve_every_change() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "BetterTaskbar-config-test-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("widgets.json");
        write(&path, &Config::default()).unwrap();
        let mut children: Vec<_> = (0..4)
            .map(|_| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "config::tests::update_worker"])
                    .env("BTW_CONFIG_TEST_PATH", &path)
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let Disk::Ok(cfg) = read(&path) else {
            panic!("config must remain readable");
        };
        assert_eq!(cfg.weather_widget.x, WidgetState::default().x + 100);
        assert_eq!(cfg.calendar, CalendarCfg::default());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn new_provider_widgets_are_opt_in_and_catalog_keys_are_unique() {
        let cfg = Config::default();
        let mut keys = std::collections::HashSet::new();
        for kind in crate::app::Kind::ALL {
            assert!(keys.insert(kind.key()));
            assert_eq!(
                kind.state(&cfg).enabled,
                !matches!(kind, crate::app::Kind::Usage(_))
            );
        }
        let cfg: Config = serde_json::from_str(r#"{"usage_widgets":{}}"#).unwrap();
        assert!(
            !crate::app::Kind::Usage(codezeno_usage_core::providers::ProviderId::Claude)
                .state(&cfg)
                .enabled
        );
    }
    #[test]
    fn rejected_transaction_does_not_publish_or_overwrite_config() {
        let dir =
            std::env::temp_dir().join(format!("BetterTaskbar-rejected-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("widgets.json");
        let cfg = Arc::new(Mutex::new(Config::default()));
        write(&path, &Config::default()).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(checked_update_at(&cfg, &path, |c| {
            c.mail.password = "test-only".into();
            Err("reject fixture".into())
        })
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(cfg.lock().unwrap().mail.password.is_empty());
        std::fs::write(&path, "{unfinished").unwrap();
        assert!(try_update_at(&cfg, &path, |c| c.mail.port = 1).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{unfinished");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let cfg: Config =
            serde_json::from_str(r#"{"calendar":{"username":"u","path":"/x/"}}"#).unwrap();
        assert_eq!(cfg.calendar.username, "u");
        assert_eq!(cfg.calendar.path, "/x/");
        assert_eq!(cfg.calendar.port, 8081);
        assert_eq!(cfg.mail, MailCfg::default());
        assert!(cfg.weather_widget.visible);
    }

    #[test]
    fn round_trips() {
        let cfg = Config::default();
        let text = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), cfg);
    }
}
