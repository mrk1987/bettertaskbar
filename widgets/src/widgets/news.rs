//! News pill: rotating headlines from RSS and Atom feeds, with the source
//! and the headline's age. Double-click opens the headline on screen.
//!
//! The headline stands still while the pointer is on the pill (if pause on
//! hover is on) and for a moment after any press, so the second click of a
//! double-click always opens the story the first click landed on.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use eframe::egui;

use crate::config::{Config, FeedSource, NewsCfg, SharedConfig};
use crate::icons::IconCache;
use crate::widgets::calendar::two_lines;
use crate::widgets::{Update, Widget};

/// At most this many headlines per source go into the rotation, so one busy
/// feed cannot crowd out the others.
const PER_SOURCE: usize = 6;
/// A press holds the headline for this long; longer than any double-click.
const HOLD_AFTER_PRESS: Duration = Duration::from_secs(3);
/// Feeds larger than this are refused.
const MAX_FEED_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Headline {
    pub source: String,
    pub title: String,
    pub link: String,
    pub published: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Headlines {
    pub items: Vec<Headline>,
    /// Sources that could not be read this time, by name.
    pub failed: Vec<String>,
    pub fetched: DateTime<Utc>,
}

/// One feed's entries, in feed order. Accepts RSS 2.0, RSS 1.0 (RDF) and
/// Atom; matching is by local element name, so namespaces do not matter.
pub fn parse_feed(xml: &str, source: &str) -> Result<Vec<Headline>, String> {
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| format!("not a readable feed: {e}"))?;
    let root = doc.root_element();
    let atom = root.tag_name().name() == "feed";
    let entry_name = if atom { "entry" } else { "item" };
    let child_text = |node: roxmltree::Node, name: &str| -> Option<String> {
        node.children()
            .find(|c| c.is_element() && c.tag_name().name() == name)
            .map(|c| {
                c.descendants()
                    .filter(|d| d.is_text())
                    .filter_map(|d| d.text())
                    .collect::<String>()
            })
    };
    let mut out = Vec::new();
    for entry in root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == entry_name)
    {
        let Some(title) = child_text(entry, "title").map(|t| clean_title(&t)) else {
            continue;
        };
        let link = if atom {
            // Prefer rel="alternate" (or no rel); fall back to any href.
            let links: Vec<_> = entry
                .children()
                .filter(|c| c.is_element() && c.tag_name().name() == "link")
                .collect();
            links
                .iter()
                .find(|l| l.attribute("rel").is_none_or(|r| r == "alternate"))
                .or(links.first())
                .and_then(|l| l.attribute("href"))
                .map(str::to_owned)
        } else {
            child_text(entry, "link")
                .filter(|l| !l.trim().is_empty())
                .or_else(|| {
                    // Some RSS feeds only have a permalink guid.
                    entry
                        .children()
                        .find(|c| {
                            c.is_element()
                                && c.tag_name().name() == "guid"
                                && c.attribute("isPermaLink") != Some("false")
                        })
                        .and_then(|g| g.text())
                        .map(str::to_owned)
                })
        };
        let Some(link) = link.and_then(|l| crate::click_action::web_url(&l).ok()) else {
            continue;
        };
        let published = ["pubDate", "published", "updated", "date"]
            .iter()
            .filter_map(|name| child_text(entry, name))
            .find_map(|text| parse_date(&text));
        if title.is_empty() {
            continue;
        }
        out.push(Headline {
            source: source.to_owned(),
            title,
            link,
            published,
        });
    }
    Ok(out)
}

fn parse_date(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    DateTime::parse_from_rfc2822(text)
        .or_else(|_| DateTime::parse_from_rfc3339(text))
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Titles sometimes carry markup or entities that survived XML decoding
/// (`&amp;quot;`, `<b>`). Strip tags, decode the common entities once, and
/// collapse whitespace.
fn clean_title(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut in_tag = false;
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let text = text
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Key for spotting the same story twice: the link without fragment,
/// trailing slash or tracking parameters, lowercased.
fn link_key(link: &str) -> String {
    match url::Url::parse(link) {
        Ok(mut u) => {
            u.set_fragment(None);
            let kept: Vec<(String, String)> = u
                .query_pairs()
                .filter(|(k, _)| !k.starts_with("utm_"))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            if kept.is_empty() {
                u.set_query(None);
            } else {
                u.query_pairs_mut().clear().extend_pairs(kept);
            }
            u.as_str().trim_end_matches('/').to_lowercase()
        }
        Err(_) => link.to_lowercase(),
    }
}

fn title_key(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The rotation: newest first within each source, sources taking turns,
/// duplicates (same link or same title) kept once, and nothing older than
/// `max_age` unless that would leave nothing at all.
pub fn merge(
    per_source: Vec<Vec<Headline>>,
    now: DateTime<Utc>,
    max_age: Duration,
) -> Vec<Headline> {
    let max_age = chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::days(1));
    let mut queues: Vec<Vec<Headline>> = per_source
        .into_iter()
        .map(|mut items| {
            // Stable: entries without a date keep their feed order, after dated ones.
            items.sort_by_key(|h| std::cmp::Reverse(h.published));
            items
        })
        .collect();
    let fresh = |h: &Headline| h.published.is_none_or(|p| now - p <= max_age);
    if queues.iter().flatten().any(fresh) {
        for q in &mut queues {
            q.retain(fresh);
        }
    }
    let mut seen_links = HashSet::new();
    let mut seen_titles = HashSet::new();
    let mut taken = vec![0usize; queues.len()];
    let mut cursor = vec![0usize; queues.len()];
    let mut out = Vec::new();
    loop {
        let mut progressed = false;
        for (i, q) in queues.iter().enumerate() {
            while taken[i] < PER_SOURCE && cursor[i] < q.len() {
                let h = &q[cursor[i]];
                cursor[i] += 1;
                if seen_links.insert(link_key(&h.link)) & seen_titles.insert(title_key(&h.title)) {
                    out.push(h.clone());
                    taken[i] += 1;
                    progressed = true;
                    break;
                }
            }
        }
        if !progressed {
            return out;
        }
    }
}

fn fetch_source(source: &FeedSource) -> Result<Vec<Headline>, String> {
    let url = crate::click_action::web_url(&source.url)?;
    let mut response = crate::http::agent()
        .get(&url)
        .header("User-Agent", crate::http::USER_AGENT)
        .header(
            "Accept",
            "application/rss+xml, application/atom+xml, application/xml;q=0.9, text/xml;q=0.8",
        )
        .call()
        .map_err(|e| format!("{}: {e}", source.name))?;
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_FEED_BYTES)
        .read_to_string()
        .map_err(|e| format!("{}: {e}", source.name))?;
    parse_feed(&text, &source.name).map_err(|e| format!("{}: {e}", source.name))
}

pub fn fetch(cfg: &NewsCfg) -> Result<Headlines, String> {
    let sources: Vec<&FeedSource> = cfg.feeds.iter().filter(|f| f.enabled).collect();
    if sources.is_empty() {
        return Err("No news sources are switched on".into());
    }
    // Feeds are fetched side by side: one slow outlet must not hold up the rest.
    let results: Vec<(String, Result<Vec<Headline>, String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = sources
            .iter()
            .map(|s| (s.name.clone(), scope.spawn(|| fetch_source(s))))
            .collect();
        handles
            .into_iter()
            .map(|(name, h)| {
                (
                    name,
                    h.join()
                        .unwrap_or_else(|_| Err("feed reader stopped".into())),
                )
            })
            .collect()
    });
    let mut lists = Vec::new();
    let mut failed = Vec::new();
    let mut errors = Vec::new();
    for (name, result) in results {
        match result {
            Ok(items) => lists.push(items),
            Err(e) => {
                failed.push(name);
                errors.push(e);
            }
        }
    }
    if lists.is_empty() {
        return Err(errors.join("; "));
    }
    let now = Utc::now();
    Ok(Headlines {
        items: merge(
            lists,
            now,
            Duration::from_secs(cfg.max_age_hours.max(1) * 3600),
        ),
        failed,
        fetched: now,
    })
}

/// "12 min", "3 h", "2 d"; "now" under a minute; empty without a date.
pub fn age(published: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(p) = published else {
        return String::new();
    };
    let mins = (now - p).num_minutes().max(0);
    match mins {
        0 => "now".into(),
        1..=59 => format!("{mins} min"),
        60..=2879 => format!("{} h", mins / 60),
        _ => format!("{} d", mins / 1440),
    }
}

fn pick(c: &Config) -> (NewsCfg, u64, bool) {
    // Drawing settings must not trigger a fetch.
    let mut settings = c.news.clone();
    settings.rotate_secs = 0;
    settings.pause_on_hover = false;
    (
        settings,
        c.news.refresh_secs,
        c.news.feeds.iter().any(|f| f.enabled),
    )
}

fn fetch_update(cfg: &NewsCfg) -> Result<(Headlines, String), String> {
    let h = fetch(cfg)?;
    let mut summary = format!("{} headlines", h.items.len());
    if !h.failed.is_empty() {
        summary.push_str(&format!(", unreadable: {}", h.failed.join(", ")));
    }
    Ok((h, summary))
}

pub struct NewsWidget {
    refresh: crate::widgets::RefreshRequest,
    cfg: SharedConfig,
    rx: Receiver<Update<Headlines>>,
    headlines: Option<Headlines>,
    /// Fresh headlines that arrived while the current one was held.
    pending: Option<Headlines>,
    /// The last fetch failed; what is shown is from an earlier one.
    offline: bool,
    notice: Option<String>,
    index: usize,
    last_rotate: Instant,
    /// The headline on screen has been shown in full once.
    shown: bool,
    hovered: bool,
    held_until: Option<Instant>,
}

impl NewsWidget {
    pub fn new(cfg: SharedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let refresh = crate::widgets::poll("news", cfg.clone(), tx, pick, fetch_update);
        Self {
            refresh,
            cfg,
            rx,
            headlines: None,
            pending: None,
            offline: false,
            notice: Some("Loading…".into()),
            index: 0,
            last_rotate: Instant::now(),
            shown: true,
            hovered: false,
            held_until: None,
        }
    }

    fn current(&self) -> Option<&Headline> {
        let items = &self.headlines.as_ref()?.items;
        items.get(self.index.min(items.len().saturating_sub(1)))
    }

    /// Old data or a failed refresh: say so rather than pass old news as new.
    fn stale(&self) -> bool {
        let refresh = self.cfg.lock().unwrap().news.refresh_secs.max(60);
        self.offline
            || self
                .headlines
                .as_ref()
                .is_some_and(|h| (Utc::now() - h.fetched).num_seconds() > (refresh * 3) as i64)
    }

    fn second_line(&self, h: &Headline) -> String {
        let mut parts = vec![h.source.clone()];
        let age = age(h.published, Utc::now());
        if !age.is_empty() {
            parts.push(age);
        }
        if self.stale() {
            parts.push("offline".into());
        }
        parts.join(" · ")
    }
}

/// Whether the rotation may move on now.
fn may_rotate(
    hovered: bool,
    pause_on_hover: bool,
    held_until: Option<Instant>,
    now: Instant,
) -> bool {
    !(hovered && pause_on_hover) && held_until.is_none_or(|t| now >= t)
}

fn news_tile(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, egui::Color32::from_rgb(0x2F, 0x6F, 0xD6));
    let white = egui::Color32::WHITE;
    let page = egui::Rect::from_center_size(rect.center(), egui::vec2(15.0, 13.0));
    p.rect_stroke(
        page,
        1.5,
        egui::Stroke::new(1.4, white),
        egui::StrokeKind::Middle,
    );
    // A headline block and three lines of text.
    p.rect_filled(
        egui::Rect::from_min_size(page.min + egui::vec2(2.5, 2.5), egui::vec2(4.5, 3.5)),
        0.5,
        white,
    );
    for (y, x0) in [(3.2, 8.5), (6.3, 2.5), (9.4, 2.5)] {
        p.line_segment(
            [
                egui::pos2(page.left() + x0, page.top() + y),
                egui::pos2(page.right() - 2.5, page.top() + y),
            ],
            egui::Stroke::new(1.1, white),
        );
    }
}

impl Widget for NewsWidget {
    fn size(&self) -> (f32, f32) {
        (260.0, 48.0)
    }

    fn refresh(&mut self) {
        self.refresh.request();
        self.notice = Some("Refresh requested…".into());
    }

    fn interaction(&mut self, hovered: bool, pressed: bool) {
        self.hovered = hovered;
        if pressed {
            self.held_until = Some(Instant::now() + HOLD_AFTER_PRESS);
        }
    }

    fn open_current(&mut self) -> Result<(), String> {
        let link = self
            .current()
            .map(|h| h.link.clone())
            .ok_or("No headline to open yet.")?;
        // Hold the story that was opened while the browser comes up.
        self.held_until = Some(Instant::now() + HOLD_AFTER_PRESS);
        crate::shell::try_open_url(&crate::click_action::web_url(&link)?)
    }

    fn values(&self) -> crate::theme::Values {
        let (primary, secondary, source, age_text) = match self.current() {
            Some(h) => (
                h.title.clone(),
                self.second_line(h),
                h.source.clone(),
                age(h.published, Utc::now()),
            ),
            None => (
                "News".into(),
                self.notice.clone().unwrap_or_default(),
                String::new(),
                String::new(),
            ),
        };
        [
            ("primary".into(), primary),
            ("secondary".into(), secondary),
            ("source".into(), source),
            ("age".into(), age_text),
        ]
        .into_iter()
        .collect()
    }

    fn tick(&mut self) {
        let (rotate, pause_on_hover) = {
            let c = self.cfg.lock().unwrap();
            (c.news.rotate_secs.max(3), c.news.pause_on_hover)
        };
        let now = Instant::now();
        // While the headline is held, new headlines wait: a refresh must not
        // swap the story between the two clicks of a double-click.
        let frozen = !may_rotate(self.hovered, pause_on_hover, self.held_until, now);
        while let Ok(update) = self.rx.try_recv() {
            match update {
                Update::Data(fresh) => {
                    self.pending = Some(fresh);
                    self.offline = false;
                    self.notice = None;
                }
                Update::Failed(reason) => {
                    self.offline = true;
                    self.notice = Some(reason);
                }
                Update::Unconfigured => {
                    self.headlines = None;
                    self.pending = None;
                    self.notice = Some("Switch on a news source in settings".into());
                }
            }
        }
        if !frozen {
            if let Some(fresh) = self.pending.take() {
                // Stay on the story on screen if it is still in the list.
                let current = self.current().map(|h| h.link.clone());
                let position =
                    current.and_then(|link| fresh.items.iter().position(|h| h.link == link));
                self.index = position.unwrap_or(0);
                if position.is_none() {
                    self.last_rotate = now;
                }
                self.headlines = Some(fresh);
            }
        }
        let count = self.headlines.as_ref().map_or(0, |h| h.items.len());
        if frozen {
            // Paused time does not count toward the next turn.
            self.last_rotate = now;
        } else if count > 0
            && self.shown
            && self.last_rotate.elapsed() >= Duration::from_secs(rotate)
        {
            // A headline too long for the pill scrolls past once first.
            self.index = (self.index + 1) % count;
            self.last_rotate = now;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _icons: &mut IconCache) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
            news_tile(ui);
            let (first, second) = match self.current() {
                Some(h) => (h.title.clone(), self.second_line(h)),
                None => (
                    "News".into(),
                    self.notice.clone().unwrap_or_else(|| "No headlines".into()),
                ),
            };
            self.shown = two_lines(ui, &first, &second, 200.0);
        });
    }
}

#[cfg(test)]
impl NewsWidget {
    /// Fixed sample data for the README banner (`single.rs`, `readme_banner`):
    /// no thread, no network, nothing from the user's own accounts.
    pub fn sample(cfg: SharedConfig) -> Self {
        let now = Utc::now();
        Self {
            refresh: Default::default(),
            cfg,
            rx: mpsc::channel().1,
            headlines: Some(Headlines {
                items: vec![Headline {
                    source: "World News".into(),
                    title: "Northern lights forecast for tonight".into(),
                    link: "https://example.com/".into(),
                    published: Some(now - chrono::Duration::minutes(20)),
                }],
                failed: Vec::new(),
                fetched: now,
            }),
            pending: None,
            offline: false,
            notice: None,
            index: 0,
            last_rotate: Instant::now(),
            shown: true,
            hovered: false,
            held_until: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><channel><title>T</title>
<item><title><![CDATA[First &amp; <b>bold</b>]]></title><link>https://example.com/a?utm_source=x</link>
<pubDate>Sat, 14 Nov 2026 10:00:00 +0100</pubDate></item>
<item><title>No link</title></item>
<item><title>Script link</title><link>javascript:alert(1)</link></item>
<item><title>Guid only</title><guid>https://example.com/g</guid><dc:date>2026-11-14T08:00:00Z</dc:date></item>
</channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>A</title>
<entry><title type="html">Atom story</title>
<link rel="replies" href="https://example.org/comments"/>
<link rel="alternate" href="https://example.org/story"/>
<updated>2026-11-14T09:30:00Z</updated></entry></feed>"#;

    #[test]
    fn rss_and_atom_entries_keep_only_web_links() {
        let items = parse_feed(RSS, "Ex").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "First & bold");
        assert_eq!(
            items[0].published.unwrap().to_rfc3339(),
            "2026-11-14T09:00:00+00:00"
        );
        assert_eq!(items[1].link, "https://example.com/g");
        let atom = parse_feed(ATOM, "At").unwrap();
        assert_eq!(atom[0].link, "https://example.org/story");
        assert!(atom[0].published.is_some());
        assert!(parse_feed("<html><body>oops", "x").is_err());
    }

    fn h(
        source: &str,
        title: &str,
        link: &str,
        mins_ago: Option<i64>,
        now: DateTime<Utc>,
    ) -> Headline {
        Headline {
            source: source.into(),
            title: title.into(),
            link: link.into(),
            published: mins_ago.map(|m| now - chrono::Duration::minutes(m)),
        }
    }

    #[test]
    fn rotation_alternates_sources_drops_duplicates_and_old_news() {
        let now = Utc::now();
        let a = vec![
            h("A", "Old", "https://a/old", Some(60 * 30), now),
            h("A", "A1", "https://a/1", Some(5), now),
            h("A", "A2", "https://a/2", Some(50), now),
        ];
        let b = vec![
            h("B", "Same story!", "https://b/1", Some(10), now),
            h("B", "a1", "https://b/dup-title", Some(1), now),
            h("B", "Copy", "https://a/1/?utm_medium=rss#x", Some(2), now),
        ];
        let merged = merge(vec![a, b], now, Duration::from_secs(24 * 3600));
        let titles: Vec<_> = merged.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, ["A1", "Same story!", "A2"]);
        // When everything is old, show it rather than nothing.
        let old = vec![h("A", "Old", "https://a/old", Some(60 * 30), now)];
        assert_eq!(merge(vec![old], now, Duration::from_secs(3600)).len(), 1);
    }

    #[test]
    fn ages_read_short() {
        let now = Utc::now();
        let ago = |m| Some(now - chrono::Duration::minutes(m));
        assert_eq!(age(ago(0), now), "now");
        assert_eq!(age(ago(12), now), "12 min");
        assert_eq!(age(ago(180), now), "3 h");
        assert_eq!(age(ago(3 * 1440), now), "3 d");
        assert_eq!(age(None, now), "");
    }

    #[test]
    fn a_press_or_hover_holds_the_headline() {
        let now = Instant::now();
        assert!(may_rotate(false, true, None, now));
        assert!(!may_rotate(true, true, None, now));
        assert!(may_rotate(true, false, None, now));
        assert!(!may_rotate(
            false,
            false,
            Some(now + Duration::from_secs(1)),
            now
        ));
        assert!(may_rotate(false, false, Some(now), now));
    }
}
