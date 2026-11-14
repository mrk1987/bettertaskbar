pub mod calendar;
pub mod mail;
pub mod usage;
pub mod weather;

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::config::{Config, SharedConfig};
use crate::icons::IconCache;

pub trait Widget {
    fn tick(&mut self) {}
    fn values(&self) -> crate::theme::Values {
        crate::theme::Values::default()
    }
    fn size(&self) -> (f32, f32);
    fn refresh(&mut self);
    fn ui(&mut self, ui: &mut egui::Ui, icons: &mut IconCache);
}

/// What a background fetch produced for a pill.
pub enum Update<T> {
    /// No username in the config yet.
    Unconfigured,
    Data(T),
    Failed(String),
}

/// Coalesces repeated requests and keeps manual refreshes at least 30 s
/// apart from completed readings. Pending requests survive the cooldown.
#[derive(Clone, Default)]
pub struct RefreshRequest(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl RefreshRequest {
    pub fn request(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn clear(&self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn take_if_due(&self, elapsed: Duration) -> bool {
        elapsed >= Duration::from_secs(30)
            && self.0.swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

/// Background refresh loop for a pill. `pick` takes this widget's settings and
/// its refresh interval out of the config; `fetch` does the work and returns
/// the data plus a one-line summary for the log. A fetch runs at start, every
/// `refresh` seconds, and at once when the settings change, so saving new
/// credentials shows up without waiting out the interval.
///
/// Outcomes are logged only when they change, so a bridge that is down for a
/// night leaves one line, not hundreds.
pub fn poll<C, T>(
    tag: &'static str,
    cfg: SharedConfig,
    tx: Sender<Update<T>>,
    pick: fn(&Config) -> (C, u64, bool),
    fetch: fn(&C) -> Result<(T, String), String>,
) -> RefreshRequest
where
    C: Clone + PartialEq + Send + 'static,
    T: Send + 'static,
{
    let refresh = RefreshRequest::default();
    let requested = refresh.clone();
    std::thread::spawn(move || {
        let mut last_logged = String::new();
        loop {
            requested.clear();
            let (settings, refresh, configured) = pick(&cfg.lock().unwrap());
            let (update, summary) = if !configured {
                (
                    Update::Unconfigured,
                    "not configured (no username)".to_owned(),
                )
            } else {
                match fetch(&settings) {
                    Ok((data, summary)) => (Update::Data(data), format!("ok: {summary}")),
                    Err(e) => (Update::Failed(short_reason(&e)), format!("FAILED: {e}")),
                }
            };
            if summary != last_logged {
                crate::log::line(tag, &summary);
                last_logged = summary;
            }
            if tx.send(update).is_err() {
                return;
            }
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(refresh.max(30)) {
                std::thread::sleep(Duration::from_secs(1));
                if pick(&cfg.lock().unwrap()).0 != settings
                    || requested.take_if_due(started.elapsed())
                {
                    break;
                }
            }
        }
    });
    refresh
}

/// The part of an error worth a pill's second line: the text after the last
/// "what was being done" prefix, e.g. "HTTP 401, check username and password".
fn short_reason(error: &str) -> String {
    let tail = error.rsplit(": ").next().unwrap_or(error).trim();
    let text = if tail.len() < 8 { error } else { tail };
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::short_reason;

    #[test]
    fn manual_refresh_coalesces_and_survives_the_cooldown() {
        let request = super::RefreshRequest::default();
        let worker = request.clone();
        request.request();
        request.request();
        request.request();
        assert!(!worker.take_if_due(std::time::Duration::from_secs(29)));
        assert!(worker.take_if_due(std::time::Duration::from_secs(30)));
        assert!(!worker.take_if_due(std::time::Duration::from_secs(60)));
        request.request();
        assert!(!worker.take_if_due(std::time::Duration::ZERO));
        assert!(worker.take_if_due(std::time::Duration::from_secs(30)));
    }

    #[test]
    fn reasons() {
        assert_eq!(
            short_reason("PROPFIND /caldav/: HTTP 401, check username and password"),
            "HTTP 401, check username and password"
        );
        assert_eq!(
            short_reason("cannot connect to 127.0.0.1:1143: connection refused"),
            "Connection refused"
        );
        assert_eq!(short_reason("oops"), "Oops");
    }
}
