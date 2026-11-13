//! IMAP unread rotator pill: Proton Mail icon + rotating unread subject.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::config::{Config, MailCfg, SharedConfig};
use crate::icons::IconCache;
use crate::imap::Unread;
use crate::widgets::calendar::{proton_tile, two_lines};
use crate::widgets::{Update, Widget};

pub struct MailWidget {
    cfg: SharedConfig,
    rx: Receiver<Update<Unread>>,
    unread: Option<Unread>,
    notice: Option<String>,
    index: usize,
    last_rotate: Instant,
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
        crate::widgets::poll("mail", cfg.clone(), tx, pick, fetch);
        Self {
            cfg,
            rx,
            unread: None,
            notice: Some("Loading…".into()),
            index: 0,
            last_rotate: Instant::now(),
        }
    }
}

impl Widget for MailWidget {
    fn size(&self) -> (f32, f32) {
        (230.0, 48.0)
    }

    fn url(&self) -> String {
        "https://mail.proton.me".to_owned()
    }

    fn values(&self) -> crate::theme::Values {
        let count = self.unread.as_ref().map(|u| u.count).unwrap_or(0);
        let primary = self
            .unread
            .as_ref()
            .and_then(|u| u.subjects.get(self.index))
            .cloned()
            .unwrap_or_else(|| "No unread mail".into());
        [
            ("primary".into(), primary),
            (
                "secondary".into(),
                self.notice
                    .clone()
                    .unwrap_or_else(|| format!("{count} unread")),
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
                    self.notice = Some("Set mail login in widgets.json".into());
                }
            }
        }
        let rotate = self.cfg.lock().unwrap().mail.rotate_secs.max(1);
        let subjects = self.unread.as_ref().map(|u| u.subjects.len()).unwrap_or(0);
        if subjects > 0 && self.last_rotate.elapsed() >= Duration::from_secs(rotate) {
            self.index = (self.index + 1) % subjects;
            self.last_rotate = Instant::now();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _icons: &mut IconCache) {
        let subjects = self.unread.as_ref().map(|u| u.subjects.len()).unwrap_or(0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
            proton_tile(ui, "mail");
            let subject = self.unread.as_ref().and_then(|u| {
                u.subjects
                    .get(self.index.min(subjects.saturating_sub(1)))
                    .cloned()
            });
            let (first, second) = match (&self.notice, &self.unread) {
                (Some(notice), _) => (subject.unwrap_or_else(|| "Mail".into()), notice.clone()),
                (None, Some(u)) if u.count > 0 => (
                    subject.unwrap_or_else(|| "Unread mail".into()),
                    format!("{} unread", u.count),
                ),
                (None, _) => ("No unread mail".into(), "Inbox".into()),
            };
            two_lines(ui, &first, &second, 170.0);
        });
    }
}
