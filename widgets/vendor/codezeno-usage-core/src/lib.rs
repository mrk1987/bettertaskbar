//! Provider polling adapted from CodeZeno/Claude-Code-Usage-Monitor (MIT).
#![allow(dead_code)]
mod accounts;
mod app_settings;
mod known_folders;
pub mod models;
pub mod poller;
pub mod providers;
mod winsqlite;
pub mod localization {
    #[derive(Clone, Copy)]
    pub enum LanguageId {
        English,
    }
    impl LanguageId {
        pub fn text(self, text: &str) -> &str {
            text
        }
    }
}
// Deliberately do not retain upstream diagnostic bodies: they can include
// credential paths and service replies. The host reports typed status only.
mod diagnose {
    pub fn log(_message: impl AsRef<str>) {}
    pub fn log_lazy(_message: impl FnOnce() -> String) {}
    pub fn log_error(_context: &str, _error: impl std::fmt::Display) {}
    pub fn is_enabled() -> bool {
        false
    }
}
