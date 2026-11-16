//! Yr.no weather pill: symbol icon + high/low + precipitation.

use crate::i18n::{tr, trf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::Value;

use crate::config::{Place, SharedConfig};
use crate::http::USER_AGENT;
use crate::icons::IconCache;
use crate::widgets::Widget;

const WARM: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x6B, 0x6B);
const COLD: egui::Color32 = egui::Color32::from_rgb(0x6F, 0xB7, 0xFF);
const WARM_ON_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xB0, 0x20, 0x20);
const COLD_ON_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x15, 0x60, 0xA0);

/// Above zero reads red, zero and below read blue (by the displayed value).
fn temp_color(t: f32, on_light: bool) -> egui::Color32 {
    match (t.round() > 0.0, on_light) {
        (true, false) => WARM,
        (false, false) => COLD,
        (true, true) => WARM_ON_LIGHT,
        (false, true) => COLD_ON_LIGHT,
    }
}

#[derive(Clone, Debug)]
pub struct WeatherData {
    pub hi: f32,
    pub lo: f32,
    pub precip: f32,
    pub symbol: String,
    /// Every wet hour in the hourly part of the forecast. The hint is worked
    /// out from these at drawing time, so it follows the clock between
    /// fetches: "Rain now" ends when the wet hour does.
    pub wet_hours: Vec<Onset>,
}

impl WeatherData {
    pub fn onset(&self, now: chrono::DateTime<chrono::Utc>) -> Option<&Onset> {
        current_onset(&self.wet_hours, now)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Onset {
    /// "Rain", "Sleet" or "Snow".
    pub kind: &'static str,
    /// Start of the first wet hour (may be in the past: it is wet now).
    pub at: chrono::DateTime<chrono::Utc>,
}

/// An hour counts as wet from this much precipitation, in mm.
const WET_MM: f64 = 0.1;
/// Onsets further ahead than this are not announced.
const ONSET_WINDOW_HOURS: i64 = 6;

/// The wet hours in met.no's hourly series, in order.
fn wet_hours(series: &[Value]) -> Vec<Onset> {
    let mut out = Vec::new();
    for entry in series {
        let Some(at) = entry["time"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
        else {
            continue;
        };
        let next = &entry["data"]["next_1_hours"];
        // Hourly steps end after about two days; there is nothing more to read.
        let Some(amount) = next["details"]["precipitation_amount"].as_f64() else {
            break;
        };
        if amount >= WET_MM {
            let symbol = next["summary"]["symbol_code"].as_str().unwrap_or("");
            let kind = if symbol.contains("snow") {
                "Snow"
            } else if symbol.contains("sleet") {
                "Sleet"
            } else {
                "Rain"
            };
            out.push(Onset { kind, at });
        }
    }
    out
}

/// The wet hour to announce at `now`: the first one not yet over, if it
/// starts within the window.
fn current_onset(wet: &[Onset], now: chrono::DateTime<chrono::Utc>) -> Option<&Onset> {
    wet.iter()
        .find(|o| o.at + chrono::Duration::hours(1) > now)
        .filter(|o| o.at - now <= chrono::Duration::hours(ONSET_WINDOW_HOURS))
}

#[cfg(test)]
fn find_onset(series: &[Value], now: chrono::DateTime<chrono::Utc>) -> Option<Onset> {
    current_onset(&wet_hours(series), now).cloned()
}

/// The hint for the pill's second line: "Rain now", "Rain 40m", "Snow 3h".
/// The pill's second line: when rain, sleet or snow starts within the
/// window ("Rain 40m"), or that none is due ("Dry 6h"). Always shown, so the
/// pill says what the next hours bring rather than the day's total.
fn rain_line(d: &WeatherData, now: chrono::DateTime<chrono::Utc>) -> String {
    match d.onset(now) {
        Some(o) => onset_hint(o, now),
        None => trf("Dry {hours}h", &[("hours", &ONSET_WINDOW_HOURS)]),
    }
}

pub fn onset_hint(onset: &Onset, now: chrono::DateTime<chrono::Utc>) -> String {
    let mins = (onset.at - now).num_minutes();
    // `kind` stays English in the data; it is translated only for display.
    let kind = match onset.kind {
        "Snow" => tr("Snow"),
        "Sleet" => tr("Sleet"),
        _ => tr("Rain"),
    };
    if mins <= 0 {
        trf("{kind} now", &[("kind", &kind)])
    } else if mins < 60 {
        trf("{kind} {minutes}m", &[("kind", &kind), ("minutes", &mins)])
    } else {
        trf(
            "{kind} {hours}h",
            &[("kind", &kind), ("hours", &((mins + 30) / 60))],
        )
    }
}

pub struct WeatherWidget {
    refresh: crate::widgets::RefreshRequest,
    rx: Receiver<WeatherData>,
    latest: Option<WeatherData>,
}

impl WeatherWidget {
    pub fn new(cfg: SharedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let bg = cfg.clone();
        let refresh = crate::widgets::RefreshRequest::default();
        let requested = refresh.clone();
        std::thread::spawn(move || poll_loop(bg, tx, requested));
        Self {
            refresh,
            rx,
            latest: None,
        }
    }
}

pub fn set_place(cfg: &SharedConfig, place: Place) {
    crate::config::update(cfg, |cfg| {
        if !cfg.weather.favorites.contains(&place) {
            cfg.weather.favorites.push(place.clone());
        }
        cfg.weather.place = Some(place);
    });
}

fn poll_loop(
    cfg: SharedConfig,
    tx: mpsc::Sender<WeatherData>,
    requested: crate::widgets::RefreshRequest,
) {
    let mut last: Option<Instant> = None;
    let mut last_place: Option<Place> = None;
    loop {
        let (place, refresh) = {
            let c = cfg.lock().unwrap();
            (c.weather.place.clone(), c.weather.refresh_secs)
        };
        let manual = requested.take_if_due(last.map(|t| t.elapsed()).unwrap_or(Duration::MAX));
        let due = manual || last.is_none_or(|t| t.elapsed() >= Duration::from_secs(refresh));
        if let Some(place) = place.clone() {
            if due || last_place.as_ref() != Some(&place) {
                if let Some(data) = fetch(&place) {
                    // The pill is gone (disabled in a shared process): stop.
                    if tx.send(data).is_err() {
                        return;
                    }
                }
                last = Some(Instant::now());
                last_place = Some(place);
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn fetch(place: &Place) -> Option<WeatherData> {
    let url = format!(
        "https://api.met.no/weatherapi/locationforecast/2.0/compact?lat={:.4}&lon={:.4}",
        place.lat, place.lon
    );
    let json: Value = serde_json::from_slice(&crate::http::get_isolated(&url)?).ok()?;
    let series = json["properties"]["timeseries"].as_array()?;
    if series.is_empty() {
        return None;
    }
    let mut hi = f32::MIN;
    let mut lo = f32::MAX;
    let mut precip = 0.0_f32;
    for entry in series.iter().take(24) {
        let data = &entry["data"];
        if let Some(t) = data["instant"]["details"]["air_temperature"].as_f64() {
            hi = hi.max(t as f32);
            lo = lo.min(t as f32);
        }
        if let Some(p) = data["next_1_hours"]["details"]["precipitation_amount"].as_f64() {
            precip += p as f32;
        }
    }
    let symbol = series[0]["data"]["next_1_hours"]["summary"]["symbol_code"]
        .as_str()
        .or_else(|| series[0]["data"]["next_6_hours"]["summary"]["symbol_code"].as_str())
        .unwrap_or("cloudy")
        .to_owned();
    Some(WeatherData {
        hi,
        lo,
        precip,
        symbol,
        wet_hours: wet_hours(series),
    })
}

/// The Yr symbol Theme Studio previews with: partly cloudy, day.
pub(crate) fn preview_symbol() -> String {
    symbol_svg("partlycloudy_day")
}

/// URL of Yr's icon for a met.no `symbol_code` such as `clearsky_night`,
/// `lightrainshowers_day` or `fog`. Yr numbers its icons, and the ones that
/// show the sun or moon come in day (`d`), night (`n`) and polar twilight (`m`)
/// versions: `01n.svg`. There is no plain `01.svg`.
fn symbol_svg(symbol: &str) -> String {
    let (base, variant) = match symbol.split_once('_') {
        Some((base, "day")) => (base, "d"),
        Some((base, "night")) => (base, "n"),
        Some((base, "polartwilight")) => (base, "m"),
        Some((base, _)) => (base, ""),
        None => (symbol, ""),
    };
    let num = match base {
        "clearsky" => "01",
        "fair" => "02",
        "partlycloudy" => "03",
        "cloudy" => "04",
        "rainshowers" => "05",
        "rainshowersandthunder" => "06",
        "sleetshowers" => "07",
        "snowshowers" => "08",
        "rain" => "09",
        "heavyrain" => "10",
        "heavyrainandthunder" => "11",
        "sleet" => "12",
        "snow" => "13",
        "snowandthunder" => "14",
        "fog" => "15",
        "sleetshowersandthunder" => "20",
        "snowshowersandthunder" => "21",
        "rainandthunder" => "22",
        "sleetandthunder" => "23",
        "lightrainshowersandthunder" => "24",
        "heavyrainshowersandthunder" => "25",
        "lightssleetshowersandthunder" => "26",
        "heavysleetshowersandthunder" => "27",
        "lightssnowshowersandthunder" => "28",
        "heavysnowshowersandthunder" => "29",
        "lightrainandthunder" => "30",
        "lightsleetandthunder" => "31",
        "heavysleetandthunder" => "32",
        "lightsnowandthunder" => "33",
        "heavysnowandthunder" => "34",
        "lightrainshowers" => "40",
        "heavyrainshowers" => "41",
        "lightsleetshowers" => "42",
        "heavysleetshowers" => "43",
        "lightsnowshowers" => "44",
        "heavysnowshowers" => "45",
        "lightrain" => "46",
        "lightsleet" => "47",
        "heavysleet" => "48",
        "lightsnow" => "49",
        "heavysnow" => "50",
        // An unknown code: show plain cloud rather than nothing.
        _ => return format!("{ICONS}/04.svg"),
    };
    format!("{ICONS}/{num}{variant}.svg")
}

// Yr's set for dark backgrounds; the pills and their text are dark-themed.
const ICONS: &str = "https://www.yr.no/assets/images/weather-symbols/dark-mode/default/svg";

pub fn search_places(query: &str) -> Result<Vec<Place>, String> {
    let mut response = crate::http::agent()
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query("name", query)
        .query("count", "8")
        .query("language", "en")
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| trf("Place search failed: {error}", &[("error", &e)]))?;
    let json: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| trf("Cannot read search results: {error}", &[("error", &e)]))?;
    parse_places(&json)
}

fn parse_places(json: &Value) -> Result<Vec<Place>, String> {
    // Open-Meteo omits results when no name matches.
    let Some(results) = json.get("results") else {
        return Ok(Vec::new());
    };
    let locations = results
        .as_array()
        .ok_or(tr("Search returned an unexpected reply"))?;
    Ok(locations
        .iter()
        .filter_map(|loc| {
            let name = loc["name"].as_str()?;
            let mut parts = vec![name];
            for field in ["admin1", "country"] {
                if let Some(part) = loc[field]
                    .as_str()
                    .filter(|p| !p.is_empty() && !parts.contains(p))
                {
                    parts.push(part);
                }
            }
            let lat = loc["latitude"].as_f64()?;
            let lon = loc["longitude"].as_f64()?;
            (lat.is_finite()
                && lon.is_finite()
                && (-90.0..=90.0).contains(&lat)
                && (-180.0..=180.0).contains(&lon))
            .then(|| Place {
                name: parts.join(", "),
                lat,
                lon,
            })
        })
        .take(8)
        .collect())
}

impl Widget for WeatherWidget {
    fn size(&self) -> (f32, f32) {
        // Minimum width; the pill grows with the temperatures' text.
        (76.0, 48.0)
    }

    fn refresh(&mut self) {
        self.refresh.request();
    }

    fn values(&self) -> crate::theme::Values {
        let mut values = crate::theme::Values::new();
        if let Some(d) = &self.latest {
            for (key, value) in [
                ("primary", format!("{:.0}° / {:.0}°", d.hi, d.lo)),
                ("secondary", rain_line(d, chrono::Utc::now())),
                ("high", format!("{:.0}", d.hi)),
                ("low", format!("{:.0}", d.lo)),
                ("precip", d.precip.to_string()),
                ("rain_hint", rain_line(d, chrono::Utc::now())),
                ("icon", symbol_svg(&d.symbol)),
            ] {
                values.insert(key.into(), value);
            }
        } else {
            values.insert("primary".into(), tr("Loading weather…").into());
        }
        values
    }
    fn tick(&mut self) {
        while let Ok(data) = self.rx.try_recv() {
            self.latest = Some(data);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, icons: &mut IconCache) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
            // No symbol until there is a forecast: guessing one would download
            // and show "cloudy" first.
            let symbol = self.latest.as_ref().map(|d| symbol_svg(&d.symbol));
            if let Some(tex) = symbol.and_then(|url| icons.get(ui.ctx(), &url, 64)) {
                ui.add(egui::Image::new(&tex).fit_to_exact_size(egui::vec2(28.0, 28.0)));
            } else {
                ui.add_space(28.0);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
                // The hi/lo row is text, not a control; don't reserve the
                // default interactive row height or the stack grows too tall.
                ui.spacing_mut().interact_size.y = 12.0;
                match &self.latest {
                    Some(d) => {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(5.0, 0.0);
                            ui.label(
                                egui::RichText::new(format!("{:.0}°", d.hi))
                                    .color(temp_color(d.hi, crate::app::on_light(ui)))
                                    .size(12.0),
                            );
                            ui.label(
                                egui::RichText::new(format!("{:.0}°", d.lo))
                                    .color(temp_color(d.lo, crate::app::on_light(ui)))
                                    .size(12.0),
                            );
                        });
                        let second = rain_line(d, chrono::Utc::now());
                        // One line; the pill grows to fit it.
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(second)
                                    .color(temp_color(0.0, crate::app::on_light(ui)))
                                    .size(11.0),
                            )
                            .extend(),
                        );
                    }
                    None => {
                        ui.label(egui::RichText::new("…").weak().size(12.0));
                        ui.label(egui::RichText::new(tr("weather")).weak().size(11.0));
                    }
                }
            });
        });
    }
}

#[cfg(test)]
impl WeatherWidget {
    const SAMPLE_SYMBOL: &'static str = "partlycloudy_day";

    /// Fixed sample data for the README banner (`single.rs`, `readme_banner`):
    /// no thread, no network, nothing from the user's own accounts.
    pub fn sample() -> Self {
        let soon = chrono::Utc::now() + chrono::Duration::seconds(40 * 60 + 30);
        Self {
            refresh: Default::default(),
            rx: mpsc::channel().1,
            latest: Some(WeatherData {
                hi: 12.0,
                lo: 6.0,
                precip: 1.2,
                symbol: Self::SAMPLE_SYMBOL.into(),
                wet_hours: vec![Onset {
                    kind: "Rain",
                    at: soon,
                }],
            }),
        }
    }

    /// Where the sample's weather symbol is downloaded from.
    pub fn sample_icon_url() -> String {
        symbol_svg(Self::SAMPLE_SYMBOL)
    }
}

#[cfg(test)]
mod tests {
    use super::{current_onset, find_onset, onset_hint, parse_places, symbol_svg, wet_hours};

    #[test]
    fn temperature_colours_are_readable_on_both_bars() {
        for t in [-5.0, 0.0, 0.4, 0.6, 12.0] {
            assert!(
                crate::tone::contrast(super::temp_color(t, false), [0x20; 3]) >= 3.0,
                "{t} dark"
            );
            assert!(
                crate::tone::contrast(super::temp_color(t, true), [0xF3; 3]) >= 3.0,
                "{t} light"
            );
        }
    }

    #[test]
    fn the_second_line_says_dry_when_nothing_is_due() {
        let at = |h: u32| {
            chrono::DateTime::parse_from_rfc3339(&format!("2026-11-14T{h:02}:00:00Z"))
                .unwrap()
                .to_utc()
        };
        let mut d = super::WeatherData {
            hi: 8.0,
            lo: 5.0,
            precip: 3.0,
            symbol: String::new(),
            wet_hours: vec![super::Onset {
                kind: "Rain",
                at: at(20),
            }],
        };
        assert_eq!(super::rain_line(&d, at(12)), "Dry 6h");
        assert_eq!(super::rain_line(&d, at(16)), "Rain 4h");
        d.wet_hours.clear();
        assert_eq!(super::rain_line(&d, at(16)), "Dry 6h");
    }

    #[test]
    fn rain_onset_is_announced_within_six_hours() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-14T13:20:00Z")
            .unwrap()
            .to_utc();
        let hour = |h: u32, mm: f64, sym: &str| {
            serde_json::json!({"time": format!("2026-11-14T{h:02}:00:00Z"),
                "data": {"next_1_hours": {"summary": {"symbol_code": sym},
                                          "details": {"precipitation_amount": mm}}}})
        };
        let dry = |h| hour(h, 0.0, "cloudy");
        let series = vec![hour(12, 3.0, "rain"), dry(13), hour(14, 0.4, "lightrain")];
        let onset = find_onset(&series, now).unwrap();
        assert_eq!(onset_hint(&onset, now), "Rain 40m");
        let series = vec![hour(13, 0.2, "lightsnow"), dry(14)];
        assert_eq!(
            onset_hint(&find_onset(&series, now).unwrap(), now),
            "Snow now"
        );
        let series = vec![dry(13), dry(14), hour(16, 1.0, "sleet")];
        assert_eq!(
            onset_hint(&find_onset(&series, now).unwrap(), now),
            "Sleet 3h"
        );
        let series = vec![
            dry(13),
            dry(14),
            dry(15),
            dry(16),
            dry(17),
            dry(18),
            dry(19),
            hour(20, 2.0, "rain"),
        ];
        assert_eq!(find_onset(&series, now), None);
        // Fetched while it rained 13:00-14:00 and dry after: once 14:00 has
        // passed, the same data no longer says "Rain now".
        let wet = wet_hours(&[hour(13, 1.0, "rain"), dry(14), hour(18, 1.0, "rain")]);
        let at = |h: u32, m: u32| {
            chrono::DateTime::parse_from_rfc3339(&format!("2026-11-14T{h:02}:{m:02}:00Z"))
                .unwrap()
                .to_utc()
        };
        assert_eq!(
            onset_hint(current_onset(&wet, at(13, 50)).unwrap(), at(13, 50)),
            "Rain now"
        );
        assert_eq!(
            onset_hint(current_onset(&wet, at(14, 5)).unwrap(), at(14, 5)),
            "Rain 4h"
        );
        assert_eq!(current_onset(&wet[..1], at(14, 5)), None);
    }

    #[test]
    fn place_search_handles_empty_results_and_disambiguates_names() {
        assert!(parse_places(&serde_json::json!({})).unwrap().is_empty());
        let places = parse_places(&serde_json::json!({"results":[
            {"name":"Oslo","admin1":"Oslo","country":"Norway","latitude":59.91,"longitude":10.75},
            {"name":"Oslo","admin1":"Minnesota","country":"United States","latitude":48.19,"longitude":-97.13},
            {"name":"Invalid","latitude":1000,"longitude":0}
        ]})).unwrap();
        assert_eq!(places.len(), 2);
        assert_eq!(places[0].name, "Oslo, Norway");
        assert_eq!(places[1].name, "Oslo, Minnesota, United States");
    }

    #[test]
    fn icon_names() {
        let name = |code: &str| symbol_svg(code).rsplit('/').next().unwrap().to_owned();
        assert_eq!(name("clearsky_night"), "01n.svg");
        assert_eq!(name("fair_day"), "02d.svg");
        assert_eq!(name("partlycloudy_polartwilight"), "03m.svg");
        assert_eq!(name("cloudy"), "04.svg");
        assert_eq!(name("lightrainshowers_day"), "40d.svg");
        assert_eq!(name("lightrain"), "46.svg");
        assert_eq!(name("heavysnow"), "50.svg");
        assert_eq!(name("fog"), "15.svg");
        assert_eq!(name("somethingnew_day"), "04.svg");
    }
}
