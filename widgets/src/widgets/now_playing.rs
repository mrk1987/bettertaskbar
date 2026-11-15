//! Now Playing pill: the track, artist and playback state of the media
//! session Windows shows in its own media flyout (Spotify, browsers, the
//! Media Player app and anything else that reports to the system).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

use crate::icons::IconCache;
use crate::widgets::calendar::{compact_width, two_lines_fixed};
use crate::widgets::Widget;

/// Room the play/pause and next buttons take after the text column.
const BUTTON_GAP: f32 = 2.0;
const BUTTONS: f32 = BUTTON_GAP + 22.0 + BUTTON_GAP + 22.0;

/// How often the session is read. Reading is a few local COM calls.
const POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub state: PlayState,
    /// The player's AppUserModelID, used to bring it forward.
    pub app: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
}

impl PlayState {
    fn label(self) -> &'static str {
        match self {
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
        }
    }
}

pub struct NowPlayingWidget {
    rx: Receiver<Option<Track>>,
    track: Option<Track>,
    loaded: bool,
    /// Cleared when the pill goes away, so the reader thread stops even
    /// while nothing changes and it has nothing to send.
    alive: Arc<AtomicBool>,
    /// Where the play/pause and next buttons were drawn last frame.
    buttons: Vec<egui::Rect>,
    /// Button presses, carried out by the reader thread, which owns the
    /// media session.
    commands: mpsc::Sender<Button>,
    /// The pill's egui context, so the reader can redraw it as soon as a
    /// reading arrives rather than at the next idle frame.
    repaint: Arc<std::sync::Mutex<Option<egui::Context>>>,
}

impl NowPlayingWidget {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let running = alive.clone();
        let (commands, pressed) = mpsc::channel();
        let repaint = Arc::new(std::sync::Mutex::new(None::<egui::Context>));
        let wake = repaint.clone();
        std::thread::spawn(move || poll_loop(tx, running, pressed, wake));
        Self {
            rx,
            track: None,
            loaded: false,
            alive,
            buttons: Vec::new(),
            commands,
            repaint,
        }
    }
}

impl Drop for NowPlayingWidget {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
    }
}

fn poll_loop(
    tx: mpsc::Sender<Option<Track>>,
    alive: Arc<AtomicBool>,
    pressed: Receiver<Button>,
    wake: Arc<std::sync::Mutex<Option<egui::Context>>>,
) {
    // WinRT needs COM on this thread; the multithreaded apartment needs no
    // message loop.
    let _ = unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        )
    };
    let mut manager: Option<Manager> = None;
    let mut last: Option<Option<Track>> = None;
    let mut last_error = String::new();
    while alive.load(Ordering::SeqCst) {
        if manager.is_none() {
            match Manager::RequestAsync().and_then(|op| op.join()) {
                Ok(m) => manager = Some(m),
                Err(e) => {
                    let text = e.to_string();
                    if text != last_error {
                        crate::log::line("now-playing", &format!("no media sessions: {text}"));
                        last_error = text;
                    }
                }
            }
        }
        let current = manager.as_ref().and_then(read);
        if last.as_ref() != Some(&current) {
            if tx.send(current.clone()).is_err() {
                return;
            }
            if let Some(ctx) = wake.lock().unwrap().as_ref() {
                ctx.request_repaint();
            }
            last = Some(current);
        }
        // Wait for the next reading, or act on a button at once.
        match pressed.recv_timeout(POLL) {
            Ok(button) => {
                let before = manager.as_ref().and_then(read);
                control(manager.as_ref(), button);
                // Players take a moment to switch. Read again quickly until
                // the change shows (up to 2 s), then report even an
                // unchanged state, so the guess shown meanwhile is corrected
                // and never undone by a reading taken too early.
                let started = std::time::Instant::now();
                while started.elapsed() < Duration::from_secs(2)
                    && manager.as_ref().and_then(read) == before
                {
                    std::thread::sleep(Duration::from_millis(100));
                }
                last = None;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Carry out a button through the media session Windows shows in its own
/// media flyout. Should there be none, or the player refuse, fall back to
/// the keyboard's media key.
fn control(manager: Option<&Manager>, button: Button) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE};
    let session = manager.and_then(|m| m.GetCurrentSession().ok());
    let result = session.as_ref().map(|s| match button {
        Button::PlayPause => s.TryTogglePlayPauseAsync().and_then(|op| op.join()),
        Button::Next => s.TrySkipNextAsync().and_then(|op| op.join()),
    });
    let (name, key) = match button {
        Button::PlayPause => ("play/pause", VK_MEDIA_PLAY_PAUSE),
        Button::Next => ("next track", VK_MEDIA_NEXT_TRACK),
    };
    match result {
        Some(Ok(true)) => crate::log::line("now-playing", &format!("{name}: done by the player")),
        other => {
            let why = match other {
                None => "no media session".to_string(),
                Some(Ok(_)) => "the player refused".to_string(),
                Some(Err(e)) => e.to_string(),
            };
            crate::log::line(
                "now-playing",
                &format!("{name}: {why}; sending the media key"),
            );
            press_media_key(key);
        }
    }
}

/// One reading, for `--check=now-playing`.
pub fn read_once() -> Result<Option<Track>, String> {
    let _ = unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        )
    };
    let manager = Manager::RequestAsync()
        .and_then(|op| op.join())
        .map_err(|e| format!("no media session manager: {e}"))?;
    Ok(read(&manager))
}

/// The current session, or `None` when nothing reports media.
fn read(manager: &Manager) -> Option<Track> {
    let session = manager.GetCurrentSession().ok()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = props.Title().ok()?.to_string_lossy();
    let artist = props
        .Artist()
        .map(|a| a.to_string_lossy())
        .unwrap_or_default();
    let state = match session.GetPlaybackInfo().ok()?.PlaybackStatus().ok()? {
        Status::Playing => PlayState::Playing,
        Status::Paused => PlayState::Paused,
        _ => PlayState::Stopped,
    };
    let app = session
        .SourceAppUserModelId()
        .map(|a| a.to_string_lossy())
        .unwrap_or_default();
    (!title.trim().is_empty()).then(|| Track {
        title: title.trim().to_owned(),
        artist: artist.trim().to_owned(),
        state,
        app,
    })
}

/// Second line: artist and state, e.g. "Aurora · Paused".
fn detail(track: &Track) -> String {
    match (track.artist.is_empty(), track.state) {
        (true, state) => state.label().into(),
        (false, PlayState::Playing) => track.artist.clone(),
        (false, state) => format!("{} · {}", track.artist, state.label()),
    }
}

/// A media key, as the keyboard's own play/pause and next keys send it.
/// Windows hands it to the media session it shows in its flyout, so it
/// works with any player the keys work with.
fn press_media_key(key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };
    let event = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [event(Default::default()), event(KEYEVENTF_KEYUP)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        crate::log::line(
            "now-playing",
            &format!(
                "cannot send media key {:#x}: {}",
                key.0,
                windows::core::Error::from_thread()
            ),
        );
    } else {
        crate::log::line("now-playing", &format!("sent media key {:#x}", key.0));
    }
}

#[derive(Clone, Copy)]
enum Button {
    PlayPause,
    Next,
}

/// Draw one round media button; true when it was clicked.
fn media_button(ui: &mut egui::Ui, button: Button, playing: bool) -> (egui::Rect, bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
    let (hover, clicked) = ui.input(|i| {
        let inside = |p: Option<egui::Pos2>| p.is_some_and(|p| rect.contains(p));
        (
            inside(i.pointer.hover_pos()),
            // egui clears press_origin on the release frame; a click (a
            // press and release without dragging) released here is ours.
            i.pointer.primary_clicked() && inside(i.pointer.interact_pos()),
        )
    });
    let p = ui.painter();
    let c = rect.center();
    if hover {
        p.circle_filled(c, 11.0, egui::Color32::from_white_alpha(28));
    }
    let white = egui::Color32::from_gray(235);
    let triangle = |left: f32, w: f32, h: f32| {
        vec![
            egui::pos2(left, c.y - h),
            egui::pos2(left + w, c.y),
            egui::pos2(left, c.y + h),
        ]
    };
    match button {
        // Pause bars while playing, a play triangle otherwise.
        Button::PlayPause if playing => {
            for dx in [-2.5, 2.5] {
                p.rect_filled(
                    egui::Rect::from_center_size(c + egui::vec2(dx, 0.0), egui::vec2(2.6, 10.0)),
                    0.5,
                    white,
                );
            }
        }
        Button::PlayPause => {
            p.add(egui::Shape::convex_polygon(
                triangle(c.x - 3.5, 8.5, 5.5),
                white,
                egui::Stroke::NONE,
            ));
        }
        // Next: a triangle and a bar.
        Button::Next => {
            p.add(egui::Shape::convex_polygon(
                triangle(c.x - 5.0, 7.5, 5.0),
                white,
                egui::Stroke::NONE,
            ));
            p.rect_filled(
                egui::Rect::from_center_size(egui::pos2(c.x + 3.8, c.y), egui::vec2(2.2, 10.0)),
                0.5,
                white,
            );
        }
    }
    (rect, clicked)
}

/// An AppUserModelID safe to hand to Explorer as `shell:AppsFolder\<id>`:
/// letters, digits and the separators real IDs use, nothing that could
/// change the meaning of the argument.
fn safe_app_id(id: &str) -> Option<&str> {
    let id = id.trim();
    (!id.is_empty()
        && id.len() <= 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-!{}".contains(c)))
    .then_some(id)
}

fn music_tile(ui: &mut egui::Ui, state: Option<PlayState>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, egui::Color32::from_rgb(0xD9, 0x4F, 0x70));
    let c = rect.center();
    let white = egui::Color32::WHITE;
    // An eighth note: head, stem and flag.
    let head = c + egui::vec2(-2.5, 4.0);
    p.circle_filled(head, 3.2, white);
    let top = egui::pos2(head.x + 2.8, c.y - 7.0);
    p.line_segment(
        [egui::pos2(top.x, head.y), top],
        egui::Stroke::new(1.6, white),
    );
    p.line_segment(
        [top, top + egui::vec2(4.5, 3.5)],
        egui::Stroke::new(1.8, white),
    );
    // A paused or stopped player gets a small badge in the corner.
    if matches!(state, Some(PlayState::Paused | PlayState::Stopped)) {
        let badge = egui::Rect::from_center_size(
            rect.right_bottom() - egui::vec2(4.5, 4.5),
            egui::vec2(9.0, 9.0),
        );
        p.rect_filled(badge, 2.0, egui::Color32::from_rgb(0x20, 0x24, 0x2c));
        for dx in [-1.6, 1.6] {
            p.line_segment(
                [
                    badge.center() + egui::vec2(dx, -2.5),
                    badge.center() + egui::vec2(dx, 2.5),
                ],
                egui::Stroke::new(1.4, white),
            );
        }
    }
}

impl Widget for NowPlayingWidget {
    /// The minimum; the pill is exactly as wide as its tile, text and buttons.
    fn size(&self) -> (f32, f32) {
        (48.0, 48.0)
    }

    /// Reading is local and runs every two seconds anyway.
    fn refresh(&mut self) {}

    fn tick(&mut self) {
        while let Ok(track) = self.rx.try_recv() {
            self.track = track;
            self.loaded = true;
        }
    }

    fn values(&self) -> crate::theme::Values {
        let (primary, secondary, state) = match &self.track {
            Some(t) => (t.title.clone(), detail(t), t.state.label()),
            None => ("Nothing playing".into(), "No media".into(), ""),
        };
        [
            ("primary".into(), primary),
            ("secondary".into(), secondary),
            ("state".into(), state.into()),
            (
                "artist".into(),
                self.track
                    .as_ref()
                    .map(|t| t.artist.clone())
                    .unwrap_or_default(),
            ),
        ]
        .into_iter()
        .collect()
    }

    fn open_current(&mut self) -> Result<(), String> {
        let track = self.track.as_ref().ok_or("Nothing is playing.")?;
        let id = safe_app_id(&track.app).ok_or("The player did not say which app it is.")?;
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        std::process::Command::new(std::path::Path::new(&windir).join("explorer.exe"))
            .arg(format!("shell:AppsFolder\\{id}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Cannot open the player: {e}"))
    }

    fn ui(&mut self, ui: &mut egui::Ui, _icons: &mut IconCache) {
        {
            let mut wake = self.repaint.lock().unwrap();
            if wake.is_none() {
                *wake = Some(ui.ctx().clone());
            }
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
            music_tile(ui, self.track.as_ref().map(|t| t.state));
            let (first, second) = match (&self.track, self.loaded) {
                (Some(t), _) => (t.title.clone(), detail(t)),
                (None, true) => ("Nothing playing".into(), "No media".into()),
                (None, false) => ("Now Playing".into(), "Loading…".into()),
            };
            self.buttons.clear();
            // The same text width as the Mail and Calendar pills; longer
            // titles and artists scroll.
            let text = compact_width(ui);
            let Some(state) = self.track.as_ref().map(|t| t.state) else {
                // As wide as text plus buttons, so the pill keeps its size.
                two_lines_fixed(ui, &first, &second, text + BUTTONS);
                return;
            };
            // A fixed text column puts the buttons against the pill's right
            // edge however short the title; longer titles scroll.
            two_lines_fixed(ui, &first, &second, text);
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
            let playing = state == PlayState::Playing;
            let (rect, clicked) = media_button(ui, Button::PlayPause, playing);
            self.buttons.push(rect);
            if clicked {
                let _ = self.commands.send(Button::PlayPause);
                // Show the new state now; the next reading confirms it.
                if let Some(t) = self.track.as_mut() {
                    t.state = if playing {
                        PlayState::Paused
                    } else {
                        PlayState::Playing
                    };
                }
            }
            let (rect, clicked) = media_button(ui, Button::Next, playing);
            self.buttons.push(rect);
            if clicked {
                let _ = self.commands.send(Button::Next);
            }
        });
    }

    fn control_at(&self, pos: egui::Pos2) -> bool {
        self.buttons.iter().any(|r| r.expand(2.0).contains(pos))
    }
}

#[cfg(test)]
impl NowPlayingWidget {
    /// Fixed sample data for the README banner (`single.rs`, `readme_banner`):
    /// no thread, no network, nothing from the user's own accounts.
    pub fn sample() -> Self {
        Self {
            rx: mpsc::channel().1,
            track: Some(Track {
                title: "Northern Skies".into(),
                artist: "Aurora Fields".into(),
                state: PlayState::Playing,
                app: String::new(),
            }),
            loaded: true,
            alive: Arc::new(AtomicBool::new(true)),
            buttons: Vec::new(),
            commands: mpsc::channel().0,
            repaint: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_line_names_the_artist_and_any_pause() {
        let mut t = Track {
            title: "Song".into(),
            artist: "Band".into(),
            state: PlayState::Playing,
            app: String::new(),
        };
        assert_eq!(detail(&t), "Band");
        t.state = PlayState::Paused;
        assert_eq!(detail(&t), "Band · Paused");
        t.artist.clear();
        assert_eq!(detail(&t), "Paused");
    }

    #[test]
    fn only_plain_app_ids_reach_explorer() {
        assert_eq!(
            safe_app_id("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
            Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify")
        );
        assert_eq!(safe_app_id("Chrome"), Some("Chrome"));
        for bad in ["", "a b", "x\" /c calc", "..\\..\\cmd.exe", "a&b", "a\nb"] {
            assert_eq!(safe_app_id(bad), None, "{bad}");
        }
    }
}
