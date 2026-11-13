//! Yr.no weather pill: symbol icon + high/low + precipitation.

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

/// Above zero reads red, zero and below read blue (by the displayed value).
fn temp_color(t: f32) -> egui::Color32 {
    if t.round() > 0.0 {
        WARM
    } else {
        COLD
    }
}

#[derive(Clone, Debug)]
pub struct WeatherData {
    pub hi: f32,
    pub lo: f32,
    pub precip: f32,
    pub symbol: String,
}

pub struct WeatherWidget {
    cfg: SharedConfig,
    rx: Receiver<WeatherData>,
    latest: Option<WeatherData>,
}

impl WeatherWidget {
    pub fn new(cfg: SharedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let bg = cfg.clone();
        std::thread::spawn(move || poll_loop(bg, tx));
        Self {
            cfg,
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

fn poll_loop(cfg: SharedConfig, tx: mpsc::Sender<WeatherData>) {
    let mut last: Option<Instant> = None;
    let mut last_place: Option<Place> = None;
    loop {
        let (place, refresh) = {
            let c = cfg.lock().unwrap();
            (c.weather.place.clone(), c.weather.refresh_secs)
        };
        let due = last.is_none_or(|t| t.elapsed() >= Duration::from_secs(refresh));
        if let Some(place) = place.clone() {
            if due || last_place.as_ref() != Some(&place) {
                if let Some(data) = fetch(&place) {
                    let _ = tx.send(data);
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
    })
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
        .map_err(|e| format!("Place search failed: {e}"))?;
    let json: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("Cannot read search results: {e}"))?;
    parse_places(&json)
}

fn parse_places(json: &Value) -> Result<Vec<Place>, String> {
    // Open-Meteo omits results when no name matches.
    let Some(results) = json.get("results") else {
        return Ok(Vec::new());
    };
    let locations = results
        .as_array()
        .ok_or("Search returned an unexpected reply")?;
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

    fn url(&self) -> String {
        let cfg = self.cfg.lock().unwrap();
        match &cfg.weather.place {
            Some(p) => format!(
                "https://www.yr.no/en/forecast/daily-table/{:.4},{:.4}",
                p.lat, p.lon
            ),
            None => "https://www.yr.no/".to_owned(),
        }
    }

    fn values(&self) -> crate::theme::Values {
        let mut values = crate::theme::Values::new();
        if let Some(d) = &self.latest {
            for (key, value) in [
                ("primary", format!("{:.0}° / {:.0}°", d.hi, d.lo)),
                ("secondary", format!("{:.0} mm", d.precip)),
                ("high", format!("{:.0}", d.hi)),
                ("low", format!("{:.0}", d.lo)),
                ("precip", d.precip.to_string()),
                ("icon", symbol_svg(&d.symbol)),
            ] {
                values.insert(key.into(), value);
            }
        } else {
            values.insert("primary".into(), "Loading weather…".into());
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
                                    .color(temp_color(d.hi))
                                    .size(12.0),
                            );
                            ui.label(
                                egui::RichText::new(format!("{:.0}°", d.lo))
                                    .color(temp_color(d.lo))
                                    .size(12.0),
                            );
                        });
                        ui.label(
                            egui::RichText::new(format!("{:.0}mm", d.precip))
                                .color(COLD)
                                .size(11.0),
                        );
                    }
                    None => {
                        ui.label(egui::RichText::new("…").weak().size(12.0));
                        ui.label(egui::RichText::new("weather").weak().size(11.0));
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_places, symbol_svg};

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
