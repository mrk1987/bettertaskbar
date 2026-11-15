//! IMAP unread rotator pill: Proton Mail icon + rotating unread subject.

use crate::i18n::{tr, trf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::config::{Config, MailCfg, SharedConfig};
use crate::icons::IconCache;
use crate::imap::Unread;
use crate::widgets::calendar::{compact_width, two_lines_fixed};
use crate::widgets::{Update, Widget};

pub struct MailWidget {
    refresh: crate::widgets::RefreshRequest,
    cfg: SharedConfig,
    rx: Receiver<Update<Unread>>,
    unread: Option<Unread>,
    notice: Option<String>,
    index: usize,
    last_rotate: Instant,
    /// The subject on screen has been shown in full once.
    shown: bool,
}

fn pick(c: &Config) -> (MailCfg, u64, bool) {
    // rotate_secs only affects drawing; changing it must not trigger a fetch.
    let mut settings = c.mail.clone();
    settings.rotate_secs = 0;
    (settings, c.mail.refresh_secs, !c.mail.username.is_empty())
}

fn fetch(cfg: &MailCfg) -> Result<(Unread, String), String> {
    let unread = crate::imap::fetch(cfg)?;
    let summary = format!("{} unread", unread.count);
    Ok((unread, summary))
}

impl MailWidget {
    pub fn new(cfg: SharedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let refresh = crate::widgets::poll("mail", cfg.clone(), tx, pick, fetch);
        Self {
            refresh,
            cfg,
            rx,
            unread: None,
            notice: Some(tr("Loading…").into()),
            index: 0,
            last_rotate: Instant::now(),
            shown: true,
        }
    }
}

impl Widget for MailWidget {
    /// The minimum; the pill is as wide as its tile and `compact_width`.
    fn size(&self) -> (f32, f32) {
        (48.0, 48.0)
    }

    fn refresh(&mut self) {
        self.refresh.request();
        self.notice = Some(tr("Refresh requested…").into());
    }

    fn values(&self) -> crate::theme::Values {
        let count = self.unread.as_ref().map(|u| u.count).unwrap_or(0);
        let primary = self
            .unread
            .as_ref()
            .and_then(|u| u.subjects.get(self.index))
            .cloned()
            .unwrap_or_else(|| tr("No unread mail").into());
        [
            ("icon".into(), crate::builtin_icons::Icon::Mail.key().into()),
            ("primary".into(), primary),
            (
                "secondary".into(),
                self.notice
                    .clone()
                    .unwrap_or_else(|| trf("{count} unread", &[("count", &count)])),
            ),
            ("count".into(), count.to_string()),
        ]
        .into_iter()
        .collect()
    }
    fn tick(&mut self) {
        while let Ok(update) = self.rx.try_recv() {
            match update {
                Update::Data(unread) => {
                    // Keep the place in the rotation unless the list changed.
                    if self.unread.as_ref() != Some(&unread) {
                        self.index = 0;
                        self.last_rotate = Instant::now();
                    }
                    self.unread = Some(unread);
                    self.notice = None;
                }
                Update::Failed(reason) => self.notice = Some(reason),
                Update::Unconfigured => {
                    self.unread = None;
                    self.notice = Some(tr("Set mail login in widgets.json").into());
                }
            }
        }
        let rotate = self.cfg.lock().unwrap().mail.rotate_secs.max(1);
        let subjects = self.unread.as_ref().map(|u| u.subjects.len()).unwrap_or(0);
        // A subject too long for the pill scrolls past once before the next.
        if subjects > 0 && self.shown && self.last_rotate.elapsed() >= Duration::from_secs(rotate) {
            self.index = (self.index + 1) % subjects;
            self.last_rotate = Instant::now();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _icons: &mut IconCache) {
        let subjects = self.unread.as_ref().map(|u| u.subjects.len()).unwrap_or(0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
            crate::builtin_icons::tile(ui, crate::builtin_icons::Icon::Mail);
            let subject = self.unread.as_ref().and_then(|u| {
                u.subjects
                    .get(self.index.min(subjects.saturating_sub(1)))
                    .cloned()
            });
            let (first, second) = match (&self.notice, &self.unread) {
                (Some(notice), _) => (subject.unwrap_or_else(|| tr("Mail").into()), notice.clone()),
                (None, Some(u)) if u.count > 0 => (
                    subject.unwrap_or_else(|| tr("Unread mail").into()),
                    trf("{count} unread", &[("count", &u.count)]),
                ),
                (None, _) => (tr("No unread mail").into(), tr("Inbox").into()),
            };
            self.shown = two_lines_fixed(ui, &first, &second, compact_width(ui));
        });
    }
}

#[cfg(test)]
impl MailWidget {
    /// Fixed sample data for the README banner (`single.rs`, `readme_banner`):
    /// no thread, no network, nothing from the user's own accounts.
    pub fn sample(cfg: SharedConfig) -> Self {
        Self {
            refresh: Default::default(),
            cfg,
            rx: mpsc::channel().1,
            unread: Some(Unread {
                count: 0,
                subjects: Vec::new(),
            }),
            notice: None,
            index: 0,
            last_rotate: Instant::now(),
            shown: true,
        }
    }
}
