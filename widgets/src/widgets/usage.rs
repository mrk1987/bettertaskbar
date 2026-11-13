use super::Widget;
use crate::{config::SharedConfig, icons::IconCache, theme::Values};
use codezeno_usage_core::{models::UsageData, providers::ProviderId};
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub data: Option<UsageData>,
    pub error: Option<String>,
    pub updated: u64,
}
fn snapshot_path(provider: ProviderId) -> std::path::PathBuf {
    crate::config::dir()
        .join("usage-state")
        .join(format!("{}.json", provider.descriptor().key))
}
pub fn status(provider: ProviderId) -> String {
    let Some(snapshot) = std::fs::read(snapshot_path(provider))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Snapshot>(&bytes).ok())
    else {
        return "Waiting for the first usage reading…".into();
    };
    if let Some(error) = snapshot.error {
        return error;
    }
    let age = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(snapshot.updated);
    if age > 300 {
        return "Last reading is stale. Waiting for a refresh…".into();
    }
    snapshot
        .data
        .map(|d| {
            format!(
                "Connected · {}{}",
                if d.session.available {
                    format!("Session {:.0}%", d.session.percentage)
                } else {
                    String::new()
                },
                if d.weekly.available {
                    format!(" · Long window {:.0}%", d.weekly.percentage)
                } else {
                    String::new()
                }
            )
        })
        .unwrap_or_else(|| "No usage reading yet".into())
}
fn publish(provider: ProviderId, result: &Result<UsageData, String>) {
    let snapshot = Snapshot {
        data: result.as_ref().ok().cloned(),
        error: result.as_ref().err().cloned(),
        updated: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let path = snapshot_path(provider);
    let write = || -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec(&snapshot)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    };
    if write().is_err() {
        crate::log::line("usage", "Could not save provider status");
    }
}
/// A manual refresh this soon after the last reading waits for the rest of
/// this time, so repeated double-clicks cannot become a burst of requests.
const MANUAL_REFRESH_GAP: Duration = Duration::from_secs(30);

pub struct UsageWidget {
    provider: ProviderId,
    /// Set by a double-click; the polling thread reads again when it sees it.
    refresh: std::sync::Arc<std::sync::atomic::AtomicBool>,
    rx: Receiver<Result<UsageData, String>>,
    data: Option<UsageData>,
    notice: Option<String>,
    /// Text column width for the (session, weekly) labels it was measured for.
    text_width: Option<((String, String), f32)>,
}

impl UsageWidget {
    pub fn new(provider: ProviderId, cfg: SharedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        #[cfg(debug_assertions)]
        if std::env::args().any(|a| a == "--preview") {
            let now = std::time::SystemTime::now();
            let data = UsageData {
                session: codezeno_usage_core::models::UsageSection {
                    available: true,
                    percentage: 34.0,
                    resets_at: Some(now + Duration::from_secs(8100)),
                },
                weekly: codezeno_usage_core::models::UsageSection {
                    available: true,
                    percentage: 62.0,
                    resets_at: Some(now + Duration::from_secs(273600)),
                },
                ..Default::default()
            };
            let _ = tx.send(Ok(data));
            return Self {
                provider,
                refresh: Default::default(),
                rx,
                data: None,
                notice: None,
                text_width: None,
            };
        }

        let refresh = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let wanted = refresh.clone();
        std::thread::spawn(move || loop {
            if !crate::app::Kind::Usage(provider)
                .state(&cfg.lock().unwrap())
                .enabled
            {
                return;
            }
            let result = codezeno_usage_core::poller::fetch(provider)
                .map_err(|e| e.message(codezeno_usage_core::localization::LanguageId::English));
            publish(provider, &result);
            if tx.send(result).is_err() {
                return;
            }
            // Sleep in steps, so a shorter interval chosen in the dashboard
            // takes effect without waiting out the old one.
            let started = std::time::Instant::now();
            loop {
                let elapsed = started.elapsed();
                let manual = elapsed >= MANUAL_REFRESH_GAP
                    && wanted.swap(false, std::sync::atomic::Ordering::SeqCst);
                if manual || elapsed >= cfg.lock().unwrap().usage_refresh() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            wanted.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        Self {
            provider,
            refresh,
            rx,
            data: None,
            notice: Some("Loading usage…".into()),
            text_width: None,
        }
    }
    /// Width of the text column: the widest possible session or weekly line,
    /// measured with the real font once per pair of labels, capped at 142 px.
    fn text_width(&mut self, ctx: &egui::Context) -> f32 {
        let label = |l: Option<&String>, default: &str| l.map_or(default.to_owned(), Clone::clone);
        let labels = (
            label(
                self.data.as_ref().and_then(|d| d.session_label.as_ref()),
                "Session",
            ),
            label(
                self.data.as_ref().and_then(|d| d.weekly_label.as_ref()),
                "Weekly",
            ),
        );
        if let Some((cached, width)) = &self.text_width {
            if *cached == labels {
                return *width;
            }
        }
        let measure = |size: f32| {
            move |text: &str| {
                ctx.fonts_mut(|f| {
                    f.layout_no_wrap(
                        text.to_owned(),
                        egui::FontId::proportional(size),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x
                })
            }
        };
        let width = widest(&labels.0, measure(12.0))
            .max(widest(&labels.1, measure(11.0)))
            .ceil()
            .min(142.0);
        self.text_width = Some((labels, width));
        width
    }

    fn lines(&self) -> (String, String) {
        let section = |name: &str, s: &codezeno_usage_core::models::UsageSection| {
            let remaining = s
                .resets_at
                .and_then(|t| t.duration_since(std::time::SystemTime::now()).ok())
                .map(|t| t.as_secs());
            section_text(name, s.available, s.percentage, remaining)
        };
        match (&self.data, &self.notice) {
            (Some(data), notice) => (
                section(
                    data.session_label.as_deref().unwrap_or("Session"),
                    &data.session,
                ),
                notice.clone().unwrap_or_else(|| {
                    section(
                        data.weekly_label.as_deref().unwrap_or("Weekly"),
                        &data.weekly,
                    )
                }),
            ),
            (None, notice) => (
                self.provider.descriptor().display_name.into(),
                notice.clone().unwrap_or_default(),
            ),
        }
    }
}
impl Widget for UsageWidget {
    fn size(&self) -> (f32, f32) {
        // Minimum width; the pill fits its text (see `ui`), like Weather.
        (48.0, 48.0)
    }
    /// Read the usage again now, instead of opening the provider's page.
    fn double_click(&mut self) -> bool {
        self.refresh
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.notice = Some("Refreshing…".into());
        true
    }

    fn url(&self) -> String {
        match self.provider {
            ProviderId::Claude => "https://claude.ai/settings/usage",
            ProviderId::Codex => "https://chatgpt.com/codex/settings/usage",
            ProviderId::Antigravity => "https://antigravity.google/",
            ProviderId::OpenCode => "https://opencode.ai/",
            ProviderId::Cursor => "https://cursor.com/dashboard",
            ProviderId::Grok => "https://grok.com/",
            ProviderId::Copilot => "https://github.com/settings/billing",
        }
        .into()
    }
    fn tick(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            match result {
                Ok(data) => {
                    self.data = Some(data);
                    self.notice = None;
                }
                Err(error) => self.notice = Some(error),
            }
        }
    }
    fn values(&self) -> Values {
        let (a, b) = self.lines();
        let mut values: Values = [
            ("primary".into(), a),
            ("secondary".into(), b),
            (
                "title".into(),
                self.provider.descriptor().display_name.into(),
            ),
        ]
        .into_iter()
        .collect();
        values.insert(
            "icon".into(),
            format!("provider:{}", self.provider.descriptor().key),
        );
        if let Some(data) = &self.data {
            values.insert("usage".into(), data.session.percentage.to_string());
            values.insert("weekly".into(), data.weekly.percentage.to_string());
            if let Some(reset) = data
                .session
                .resets_at
                .and_then(|t| t.duration_since(std::time::SystemTime::now()).ok())
            {
                values.insert(
                    "reset".into(),
                    format!(
                        "{}h {}m",
                        reset.as_secs() / 3600,
                        (reset.as_secs() / 60) % 60
                    ),
                );
            }
        }
        values
    }
    fn ui(&mut self, ui: &mut egui::Ui, _icons: &mut IconCache) {
        let (a, b) = self.lines();
        let width = self.text_width(ui.ctx());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 24.0), egui::Sense::hover());
            crate::provider_icons::paint(ui, rect, self.provider);
            ui.vertical(|ui| {
                // Exactly the widest line the format can produce: the pill
                // never resizes as numbers tick. Notices truncate instead.
                ui.set_width(width);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
                ui.add(egui::Label::new(egui::RichText::new(a).size(12.0)).truncate());
                ui.add(egui::Label::new(egui::RichText::new(b).size(11.0).weak()).truncate());
            });
        });
    }
}

/// One usage line, e.g. "Session: 34% · 2h 14m". `remaining` is seconds
/// until the window resets.
fn section_text(name: &str, available: bool, percentage: f64, remaining: Option<u64>) -> String {
    if !available {
        return format!("{name}: unavailable");
    }
    let reset = remaining
        .map(|s| {
            if s >= 86400 {
                format!(" · {}d {}h", s / 86400, (s / 3600) % 24)
            } else {
                format!(" · {}h {}m", s / 3600, (s / 60) % 60)
            }
        })
        .unwrap_or_default();
    format!("{name}: {percentage:.0}%{reset}")
}

/// The widest any `section_text` line for `name` can be, so the pill keeps
/// one width while the numbers change. Every field takes its most digits
/// (percent to 199, two-digit days, hours and minutes), tried with each
/// digit in case the font's digits differ in width.
fn widest(name: &str, measure: impl Fn(&str) -> f32) -> f32 {
    let mut lines = vec![format!("{name}: unavailable")];
    for d in '0'..='9' {
        let dd = format!("{d}{d}");
        for pct in [format!("1{dd}"), dd.clone()] {
            for reset in [
                String::new(),
                format!(" · {dd}d {dd}h"),
                format!(" · {dd}h {dd}m"),
            ] {
                lines.push(format!("{name}: {pct}%{reset}"));
            }
        }
    }
    lines.iter().map(|l| measure(l)).fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::{section_text, widest};

    /// A font where every digit has its own width, so the test fails if a
    /// candidate only covers one digit shape.
    fn measure(text: &str) -> f32 {
        text.chars()
            .map(|c| c.to_digit(10).map_or(7.0, |d| 6.0 + d as f32 * 0.3))
            .sum()
    }

    #[test]
    fn the_fixed_width_covers_every_line_the_format_can_produce() {
        let mut seconds = vec![0, 59, 60, 299, 300, 3599, 3600, 17999, 18000, 86399];
        seconds.extend((1..=40).map(|d| d * 86400 + 23 * 3600));
        for name in ["Session", "Weekly", "Monthly"] {
            let fixed = widest(name, measure);
            for pct in 0..=199 {
                for &left in &seconds {
                    for remaining in [None, Some(left)] {
                        let line = section_text(name, true, pct as f64, remaining);
                        assert!(measure(&line) <= fixed, "{line} is wider than {fixed}");
                    }
                }
            }
            assert!(measure(&section_text(name, false, 0.0, None)) <= fixed);
        }
    }

    #[test]
    fn lines_keep_their_existing_format() {
        assert_eq!(
            section_text("Session", true, 34.4, Some(2 * 3600 + 14 * 60)),
            "Session: 34% · 2h 14m"
        );
        assert_eq!(
            section_text("Weekly", true, 62.0, Some(3 * 86400 + 3 * 3600)),
            "Weekly: 62% · 3d 3h"
        );
        assert_eq!(section_text("Weekly", true, 5.0, None), "Weekly: 5%");
        assert_eq!(
            section_text("Weekly", false, 5.0, None),
            "Weekly: unavailable"
        );
    }
}
