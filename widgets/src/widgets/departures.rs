//! Next Departure pill: the next bus, tram, metro, train or boat from a saved
//! stop, with a live countdown, its delay, and a traffic level for the route.
//!
//! Line 1: `Bus 31 · 6 min`. Line 2: `Traffic: High`. The icon follows the
//! transport mode of the departure on screen.
//!
//! Sources:
//! - Departures and service disruptions: Entur's Journey Planner v3
//!   (national, no key, asks for an `ET-Client-Name` header).
//! - Road situations: Statens vegvesen's DATEX II situations through their
//!   open WFS service (GeoJSON, no account), filtered to the route.
//!
//! The traffic level is never derived from the bus running late. When no
//! road data is available it says Unknown, never None, so a missing source
//! cannot pass for clear roads.

use crate::i18n::{tr, trf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use eframe::egui;
use serde_json::{json, Value};

use crate::config::{Config, DeparturesCfg, SharedConfig, Stop};
use crate::icons::IconCache;
use crate::widgets::calendar::two_lines_fixed;
use crate::widgets::{Update, Widget};

const JOURNEY_PLANNER: &str = "https://api.entur.io/journey-planner/v3/graphql";
const GEOCODER: &str = "https://api.entur.io/geocoder/v1/autocomplete";
const ROAD_SITUATIONS: &str = "https://ogckart-sn1.atlas.vegvesen.no/datex_3_1/ows";
/// Entur asks every client to identify itself as `<company>-<application>`.
const CLIENT_NAME: &str = "bettertaskbar-widgets";
/// Road situations are read at most this often for one route.
const ROAD_TTL: Duration = Duration::from_secs(300);
/// Road data older than this is not trusted: the level becomes Unknown.
const ROAD_STALE: Duration = Duration::from_secs(20 * 60);
/// At most this many stops after ours count as the route.
const ROUTE_STOPS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Bus,
    Tram,
    Metro,
    Train,
    Boat,
    Other,
}

impl Mode {
    fn parse(entur: &str) -> Self {
        match entur {
            "bus" | "coach" | "trolleybus" => Self::Bus,
            "tram" => Self::Tram,
            "metro" => Self::Metro,
            "rail" => Self::Train,
            "water" => Self::Boat,
            _ => Self::Other,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Bus => tr("Bus"),
            Self::Tram => tr("Tram"),
            Self::Metro => tr("Metro"),
            Self::Train => tr("Train"),
            Self::Boat => tr("Boat"),
            Self::Other => tr("Line"),
        }
    }
    /// Runs in road traffic, so road situations matter.
    fn on_road(self) -> bool {
        matches!(self, Self::Bus | Self::Tram | Self::Other)
    }
}

/// The mode's icon: a colored tile with a white pictogram.
fn mode_icon(mode: Option<Mode>) -> crate::builtin_icons::Icon {
    use crate::builtin_icons::Transit;
    crate::builtin_icons::Icon::Transit(match mode {
        Some(Mode::Bus) => Transit::Bus,
        Some(Mode::Tram) => Transit::Tram,
        Some(Mode::Metro) => Transit::Metro,
        Some(Mode::Train) => Transit::Train,
        Some(Mode::Boat) => Transit::Boat,
        Some(Mode::Other) | None => Transit::Other,
    })
}

/// The traffic indicator. `Unknown` when there is no usable data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Unknown,
    None,
    Low,
    Medium,
    High,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => tr("Unknown"),
            Self::None => tr("None"),
            Self::Low => tr("Low"),
            Self::Medium => tr("Medium"),
            Self::High => tr("High"),
        }
    }
    fn color(self) -> egui::Color32 {
        match self {
            Self::High => egui::Color32::from_rgb(0xFF, 0x6B, 0x6B),
            Self::Medium => egui::Color32::from_rgb(0xFF, 0xB3, 0x47),
            Self::Low => egui::Color32::from_rgb(0xE8, 0xD8, 0x6A),
            Self::None => egui::Color32::from_rgb(0x7E, 0xD3, 0x8B),
            Self::Unknown => egui::Color32::from_gray(175),
        }
    }
    /// The worse of two known levels; Unknown only if both are.
    fn worst(self, other: Level) -> Level {
        self.max(other)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Departure {
    pub mode: Mode,
    pub line: String,
    pub destination: String,
    pub aimed: DateTime<Utc>,
    pub expected: DateTime<Utc>,
    pub realtime: bool,
    pub cancelled: bool,
    pub journey: String,
    /// Worst active Entur disruption on this departure, if any.
    pub disruption: Option<Level>,
}

impl Departure {
    /// Delay in whole minutes, rounded down.
    pub fn delay_minutes(&self) -> i64 {
        (self.expected - self.aimed).num_seconds().max(0) / 60
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub stop: String,
    pub departures: Vec<Departure>,
    /// Level for the first departure, for the log and the check command.
    /// The pill works it out per departure with `traffic_for`.
    pub traffic: Level,
    /// Why the level is what it is, for the log and the check command.
    pub traffic_reason: String,
    /// The road reading, for the one journey it was taken along.
    pub road: Option<RoadReading>,
}

/// Road situations along one journey's route.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadReading {
    pub journey: String,
    pub level: Level,
    pub reason: String,
    /// When Vegvesen was asked, not when it was last reused from the cache.
    pub at: Instant,
}

impl Board {
    /// The indicator for `d`. Road data counts only for the journey it was
    /// read along, and only while it is fresh; otherwise the level falls
    /// back to Unknown (or Entur's disruptions) instead of the previous
    /// bus's reading.
    pub fn traffic_for(&self, d: &Departure) -> Level {
        let road = self
            .road
            .as_ref()
            .filter(|r| r.journey == d.journey && r.at.elapsed() < ROAD_STALE)
            .map(|r| r.level);
        combine(d.mode, road, d.disruption)
    }
}

fn agent() -> ureq::Agent {
    crate::http::agent()
}

fn graphql(query: &str) -> Result<Value, String> {
    let mut response = agent()
        .post(JOURNEY_PLANNER)
        .header("ET-Client-Name", CLIENT_NAME)
        .header("User-Agent", crate::http::USER_AGENT)
        .send_json(json!({ "query": query }))
        .map_err(|e| format!("Entur: {e}"))?;
    let value: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("Entur: unreadable reply: {e}"))?;
    if let Some(message) = value["errors"][0]["message"].as_str() {
        return Err(format!("Entur: {message}"));
    }
    Ok(value)
}

/// Entur ids are `ABC:Type:123`; refuse anything else before it goes into a query.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.'))
}

pub fn search_stops(query: &str) -> Result<Vec<Stop>, String> {
    let mut response = agent()
        .get(GEOCODER)
        .query("text", query)
        .query("layers", "venue")
        .query("size", "10")
        .header("ET-Client-Name", CLIENT_NAME)
        .header("User-Agent", crate::http::USER_AGENT)
        .call()
        .map_err(|e| trf("Stop search failed: {error}", &[("error", &e)]))?;
    let json: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| trf("Cannot read stop search results: {error}", &[("error", &e)]))?;
    Ok(parse_stops(&json))
}

fn parse_stops(json: &Value) -> Vec<Stop> {
    json["features"]
        .as_array()
        .map(|features| {
            features
                .iter()
                .filter_map(|f| {
                    let id = f["properties"]["id"].as_str()?;
                    let name = f["properties"]["label"]
                        .as_str()
                        .or(f["properties"]["name"].as_str())?;
                    let lon = f["geometry"]["coordinates"][0].as_f64()?;
                    let lat = f["geometry"]["coordinates"][1].as_f64()?;
                    (id.starts_with("NSR:StopPlace:") && valid_id(id)).then(|| Stop {
                        id: id.into(),
                        name: name.into(),
                        lat,
                        lon,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_time(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Worst Entur situation that is in effect at `now`. Planned closures next
/// week do not count today.
fn disruption(situations: &Value, now: DateTime<Utc>) -> Option<Level> {
    situations
        .as_array()?
        .iter()
        .filter(|s| {
            let start = parse_time(&s["validityPeriod"]["startTime"]);
            let end = parse_time(&s["validityPeriod"]["endTime"]);
            start.is_none_or(|t| t <= now) && end.is_none_or(|t| t >= now)
        })
        .map(|s| match s["severity"].as_str().unwrap_or("") {
            "noImpact" => Level::None,
            "verySlight" | "slight" => Level::Low,
            "severe" | "verySevere" => Level::High,
            // normal, unknown, undefined
            _ => Level::Medium,
        })
        .max()
}

/// Departures that pass the line and direction filters, soonest first.
pub fn parse_departures(json: &Value, cfg: &DeparturesCfg, now: DateTime<Utc>) -> Vec<Departure> {
    let lines: Vec<String> = cfg
        .lines
        .iter()
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty())
        .collect();
    // Comma separated: a line can end at several stops in one direction.
    let directions: Vec<String> = cfg
        .direction
        .split(',')
        .map(|d| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
        .collect();
    let mut out: Vec<Departure> = json["data"]["stopPlace"]["estimatedCalls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let line = &call["serviceJourney"]["line"];
                    let code = line["publicCode"].as_str().unwrap_or("").trim().to_owned();
                    let destination = call["destinationDisplay"]["frontText"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned();
                    if !lines.is_empty() && !lines.contains(&code.to_lowercase()) {
                        return None;
                    }
                    let lower = destination.to_lowercase();
                    if !directions.is_empty() && !directions.iter().any(|d| lower.contains(d)) {
                        return None;
                    }
                    let aimed = parse_time(&call["aimedDepartureTime"])?;
                    let expected = parse_time(&call["expectedDepartureTime"]).unwrap_or(aimed);
                    let mut situations = call["situations"].as_array().cloned().unwrap_or_default();
                    situations.extend(line["situations"].as_array().cloned().unwrap_or_default());
                    Some(Departure {
                        mode: Mode::parse(line["transportMode"].as_str().unwrap_or("")),
                        line: code,
                        destination,
                        aimed,
                        expected,
                        realtime: call["realtime"].as_bool().unwrap_or(false),
                        cancelled: call["cancellation"].as_bool().unwrap_or(false),
                        journey: call["serviceJourney"]["id"].as_str().unwrap_or("").into(),
                        disruption: disruption(&Value::Array(situations), now),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|d| d.expected);
    out
}

/// Stops the journey visits from `stop` on, as (lat, lon).
fn route_from(json: &Value, stop: &str) -> Vec<(f64, f64)> {
    let quays = json["data"]["serviceJourney"]["quays"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let start = quays
        .iter()
        .position(|q| q["stopPlace"]["id"].as_str() == Some(stop))
        .unwrap_or(0);
    quays
        .iter()
        .skip(start)
        .take(ROUTE_STOPS)
        .filter_map(|q| Some((q["latitude"].as_f64()?, q["longitude"].as_f64()?)))
        .collect()
}

/// Metres on a local flat projection around `origin`; plenty for a few km.
fn project(origin: (f64, f64), p: (f64, f64)) -> (f64, f64) {
    let k = origin.0.to_radians().cos();
    (
        (p.1 - origin.1) * 111_320.0 * k,
        (p.0 - origin.0) * 110_540.0,
    )
}

fn point_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx * dx + dy * dy;
    let t = if len == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len).clamp(0.0, 1.0)
    };
    let (x, y) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (x * x + y * y).sqrt()
}

fn segments_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let cross = |o: (f64, f64), p: (f64, f64), q: (f64, f64)| {
        (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
    };
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
}

/// Shortest distance in metres between two polylines of (lat, lon) points.
/// A single point is a polyline of one.
fn distance(route: &[(f64, f64)], shape: &[(f64, f64)]) -> f64 {
    let Some(&origin) = route.first() else {
        return f64::INFINITY;
    };
    let r: Vec<_> = route.iter().map(|p| project(origin, *p)).collect();
    let s: Vec<_> = shape.iter().map(|p| project(origin, *p)).collect();
    let pairs = |v: &Vec<(f64, f64)>| -> Vec<((f64, f64), (f64, f64))> {
        if v.len() == 1 {
            vec![(v[0], v[0])]
        } else {
            v.windows(2).map(|w| (w[0], w[1])).collect()
        }
    };
    let mut best = f64::INFINITY;
    for (a, b) in pairs(&r) {
        for (c, d) in pairs(&s) {
            if segments_cross(a, b, c, d) {
                return 0.0;
            }
            best = best
                .min(point_segment(c, a, b))
                .min(point_segment(d, a, b))
                .min(point_segment(a, c, d))
                .min(point_segment(b, c, d));
        }
    }
    best
}

/// GeoJSON coordinates (lon, lat, nested to any depth) as polylines of (lat, lon).
fn shapes(geometry: &Value) -> Vec<Vec<(f64, f64)>> {
    fn walk(v: &Value, out: &mut Vec<Vec<(f64, f64)>>) {
        let Some(items) = v.as_array() else { return };
        if let (Some(lon), Some(lat)) = (
            items.first().and_then(Value::as_f64),
            items.get(1).and_then(Value::as_f64),
        ) {
            out.push(vec![(lat, lon)]);
            return;
        }
        if items.iter().all(|i| i[0].is_number()) && !items.is_empty() {
            out.push(
                items
                    .iter()
                    .filter_map(|p| Some((p[1].as_f64()?, p[0].as_f64()?)))
                    .collect(),
            );
            return;
        }
        for i in items {
            walk(i, out);
        }
    }
    let mut out = Vec::new();
    if geometry["type"] == "GeometryCollection" {
        for g in geometry["geometries"].as_array().into_iter().flatten() {
            out.extend(shapes(g));
        }
    } else {
        walk(&geometry["coordinates"], &mut out);
    }
    out
}

/// How much one road situation slows traffic, from its DATEX type and severity.
fn road_level(p: &Value) -> Level {
    let kind = p["SITUATION_TYPE"].as_str().unwrap_or("");
    let severity = p["SEVERITY"].as_str().unwrap_or("");
    let management = p["ROAD_OR_CARRIAGEWAY_OR_LANE_MANAGEMENT_TYPE"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    // Works that apply only some hours (night closures) are a planned nuisance,
    // not a jam: the open WFS layer does not say which hours, so cap them.
    let recurring = p["NUM_PERIODS"].as_u64().unwrap_or(0) > 0;
    let mut level = match severity {
        "highest" | "high" => Level::High,
        "medium" => Level::Medium,
        _ => Level::Low,
    };
    match kind {
        "Accident"
        | "AbnormalTraffic"
        | "VehicleObstruction"
        | "GeneralObstruction"
        | "InfrastructureDamageObstruction"
        | "EnvironmentalObstruction"
        | "AnimalPresenceObstruction" => level = level.max(Level::Medium),
        _ => {}
    }
    if management.contains("closed") || management.contains("closure") {
        level = level.max(Level::High);
    }
    if recurring {
        level = level.min(Level::Low);
    }
    level
}

/// Worst road situation within `radius` metres of the route, from a
/// GeoJSON FeatureCollection of active situations.
pub fn road_traffic(geojson: &Value, route: &[(f64, f64)], radius: f64) -> (Level, String) {
    let mut worst = Level::None;
    let mut reason = "no road situations along the route".to_owned();
    for f in geojson["features"].as_array().into_iter().flatten() {
        let p = &f["properties"];
        if p["ACTIVE"].as_i64().is_some_and(|a| a != 1) {
            continue;
        }
        let near = shapes(&f["geometry"])
            .iter()
            .any(|shape| distance(route, shape) <= radius);
        if !near {
            continue;
        }
        let level = road_level(p);
        if level > worst || reason.starts_with("no ") {
            worst = worst.max(level);
            reason = format!(
                "{} ({}): {}",
                p["SITUATION_TYPE"].as_str().unwrap_or("situation"),
                p["SEVERITY"].as_str().unwrap_or("?"),
                p["DESCRIPTION"]
                    .as_str()
                    .or(p["LOCATION_DESCRIPTION"].as_str())
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect::<String>()
            );
        }
    }
    (worst, reason)
}

fn fetch_road(route: &[(f64, f64)], radius: f64) -> Result<(Level, String), String> {
    if route.is_empty() {
        return Err("route has no coordinates".into());
    }
    // Bounding box around the route, widened by the radius (in degrees).
    let pad_lat = radius / 110_540.0;
    let pad_lon = radius / (111_320.0 * route[0].0.to_radians().cos());
    let min_lat = route.iter().map(|p| p.0).fold(f64::MAX, f64::min) - pad_lat;
    let max_lat = route.iter().map(|p| p.0).fold(f64::MIN, f64::max) + pad_lat;
    let min_lon = route.iter().map(|p| p.1).fold(f64::MAX, f64::min) - pad_lon;
    let max_lon = route.iter().map(|p| p.1).fold(f64::MIN, f64::max) + pad_lon;
    let filter =
        format!("ACTIVE=1 AND BBOX(GEOM,{min_lon:.5},{min_lat:.5},{max_lon:.5},{max_lat:.5})");
    let mut response = agent()
        .get(ROAD_SITUATIONS)
        .query("service", "WFS")
        .query("version", "1.0.0")
        .query("request", "GetFeature")
        .query("typeName", "datex_3_1:SituationSimple_v2")
        .query("outputFormat", "application/json")
        .query("cql_filter", &filter)
        .header("User-Agent", crate::http::USER_AGENT)
        .call()
        .map_err(|e| format!("Vegvesen: {e}"))?;
    let json: Value = response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_json()
        .map_err(|e| format!("Vegvesen: unreadable reply: {e}"))?;
    if json["features"].as_array().is_none() {
        return Err("Vegvesen: reply has no situations list".into());
    }
    Ok(road_traffic(&json, route, radius))
}

/// Last road reading per route (line + destination), so a refresh every
/// minute does not ask Vegvesen every minute.
type CachedRoad = Result<(Level, String), String>;
static ROAD_CACHE: Mutex<Option<(String, Instant, CachedRoad)>> = Mutex::new(None);

/// Road situations along `dep`'s route, and when they were read. A new
/// journey, stop or radius is read afresh.
fn road_for(dep: &Departure, stop: &str, radius: f64) -> Result<RoadReading, String> {
    let key = format!("{stop}|{}|{radius}", dep.journey);
    let reading = |(level, reason): (Level, String), at| RoadReading {
        journey: dep.journey.clone(),
        level,
        reason,
        at,
    };
    if let Some((k, at, result)) = ROAD_CACHE.lock().unwrap().as_ref() {
        if *k == key && at.elapsed() < ROAD_TTL {
            return result.clone().map(|r| reading(r, *at));
        }
    }
    let result = (|| {
        if !valid_id(&dep.journey) {
            return Err("departure has no journey id".to_owned());
        }
        let json = graphql(&format!(
            "{{ serviceJourney(id: \"{}\") {{ quays {{ stopPlace {{ id }} latitude longitude }} }} }}",
            dep.journey
        ))?;
        fetch_road(&route_from(&json, stop), radius)
    })();
    let at = Instant::now();
    *ROAD_CACHE.lock().unwrap() = Some((key, at, result.clone()));
    result.map(|r| reading(r, at))
}

/// Combine road and Entur data into the indicator. Road modes need road
/// data to say None; without it a disruption still shows, otherwise Unknown.
pub fn combine(mode: Mode, road: Option<Level>, disruption: Option<Level>) -> Level {
    if mode.on_road() {
        match (road, disruption) {
            (Some(r), d) => r.worst(d.unwrap_or(Level::None)),
            (None, Some(d)) if d > Level::None => d,
            _ => Level::Unknown,
        }
    } else {
        // Rails and water: service disruptions are the relevant data, and
        // Entur answered, so no disruption is a known None.
        disruption.unwrap_or(Level::None)
    }
}

pub fn fetch(cfg: &DeparturesCfg) -> Result<Board, String> {
    let stop = cfg.stop.as_ref().ok_or(tr("No stop chosen"))?;
    if !valid_id(&stop.id) {
        return Err(tr("The saved stop id is not an Entur id").into());
    }
    let json = graphql(&format!(
        "{{ stopPlace(id: \"{}\") {{ name estimatedCalls(numberOfDepartures: 40, timeRange: 14400) {{ \
         realtime cancellation aimedDepartureTime expectedDepartureTime \
         destinationDisplay {{ frontText }} \
         serviceJourney {{ id line {{ publicCode transportMode situations {{ severity validityPeriod {{ startTime endTime }} }} }} }} \
         situations {{ severity validityPeriod {{ startTime endTime }} }} }} }} }}",
        stop.id
    ))?;
    if json["data"]["stopPlace"].is_null() {
        return Err(trf(
            "Entur does not know stop {stop}",
            &[("stop", &stop.id)],
        ));
    }
    let now = Utc::now();
    let departures = parse_departures(&json, cfg, now);
    let first = departures.iter().find(|d| !d.cancelled);
    let mut road = None;
    let (traffic, traffic_reason) = match first {
        None => (Level::Unknown, "no departure to assess".to_owned()),
        Some(dep) => {
            road = if dep.mode.on_road() && cfg.road_traffic {
                match road_for(
                    dep,
                    &stop.id,
                    f64::from(cfg.traffic_radius_m.clamp(50, 2000)),
                ) {
                    Ok(reading) => Some(reading),
                    Err(e) => {
                        crate::log::line("departures", &format!("road situations: {e}"));
                        None
                    }
                }
            } else {
                None
            };
            let level = combine(dep.mode, road.as_ref().map(|r| r.level), dep.disruption);
            let why = match (&road, dep.disruption) {
                (Some(r), Some(d)) => {
                    format!("road: {}; Entur disruption: {}", r.reason, d.label())
                }
                (Some(r), None) => format!("road: {}", r.reason),
                (None, Some(d)) => format!("Entur disruption: {}", d.label()),
                (None, None) if dep.mode.on_road() => "no road data".into(),
                (None, None) => "no service disruptions".into(),
            };
            (level, why)
        }
    };
    Ok(Board {
        stop: json["data"]["stopPlace"]["name"]
            .as_str()
            .unwrap_or(&stop.name)
            .to_owned(),
        departures,
        traffic,
        traffic_reason,
        road,
    })
}

/// "now", "6 min", or the clock time an hour or more ahead.
pub fn countdown(expected: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (expected - now).num_seconds();
    if secs < 60 {
        tr("now").into()
    } else if secs < 3600 {
        trf("{minutes} min", &[("minutes", &(secs / 60))])
    } else {
        expected.with_timezone(&Local).format("%H:%M").to_string()
    }
}

/// Line 1 for a departure, e.g. `Bus 31 · 6 min +3`.
/// The pill's text width: room for its usual content, "Traffic: Unknown"
/// under a two-digit countdown, so the pill keeps one size. Delays, long line
/// names and notices scroll.
fn text_width(ui: &egui::Ui) -> f32 {
    let measure = |text: &str, size: f32| {
        ui.ctx().fonts_mut(|f| {
            f.layout_no_wrap(
                text.into(),
                egui::FontId::proportional(size),
                egui::Color32::WHITE,
            )
            .size()
            .x
        })
    };
    measure(tr("Traffic: Unknown"), 11.0)
        .max(measure("31 · 10 min", 12.0))
        .ceil()
}

/// Line one of the pill, e.g. "31 · 6 min". The tile beside it shows the
/// mode, so the mode is named only for a departure without a line number.
pub fn headline(d: &Departure, now: DateTime<Utc>) -> String {
    let name = if d.line.is_empty() {
        d.mode.label().to_owned()
    } else {
        d.line.clone()
    };
    if d.cancelled {
        return trf("{line} · cancelled", &[("line", &name)]);
    }
    let mut text = format!("{name} · {}", countdown(d.expected, now));
    let delay = d.delay_minutes();
    if delay >= 2 {
        text.push_str(&format!(" +{delay}"));
    }
    text
}

fn pick(c: &Config) -> (DeparturesCfg, u64, bool) {
    (
        c.departures.clone(),
        c.departures.refresh_secs,
        c.departures.stop.is_some(),
    )
}

fn fetch_update(cfg: &DeparturesCfg) -> Result<(Board, String), String> {
    let board = fetch(cfg)?;
    let summary = format!(
        "{} departures from {}, traffic {} ({})",
        board.departures.len(),
        board.stop,
        board.traffic.label(),
        board.traffic_reason
    );
    Ok((board, summary))
}

pub struct DeparturesWidget {
    refresh: crate::widgets::RefreshRequest,
    rx: Receiver<Update<Board>>,
    board: Option<Board>,
    fetched: Option<Instant>,
    notice: Option<String>,
}

impl DeparturesWidget {
    pub fn new(cfg: SharedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let refresh = crate::widgets::poll("departures", cfg, tx, pick, fetch_update);
        Self {
            refresh,
            rx,
            board: None,
            fetched: None,
            notice: Some(tr("Loading…").into()),
        }
    }

    /// The next departure that has not left yet (30 s grace).
    fn next(&self, now: DateTime<Utc>) -> Option<&Departure> {
        self.board
            .as_ref()?
            .departures
            .iter()
            .find(|d| (d.expected - now).num_seconds() > -30 && !(d.cancelled && d.expected < now))
    }

    /// The level for the departure on screen. Old Entur data is not trusted
    /// either; old road data is dropped inside `traffic_for`.
    fn traffic(&self) -> Level {
        match (&self.board, self.fetched, self.next(Utc::now())) {
            (Some(b), Some(t), Some(d)) if t.elapsed() < ROAD_STALE => b.traffic_for(d),
            _ => Level::Unknown,
        }
    }

    fn lines(&self) -> (Option<Mode>, String, String) {
        let now = Utc::now();
        match (self.next(now), &self.notice) {
            (Some(d), None) => (
                Some(d.mode),
                headline(d, now),
                trf("Traffic: {level}", &[("level", &self.traffic().label())]),
            ),
            (Some(d), Some(n)) => (Some(d.mode), headline(d, now), n.clone()),
            (None, Some(n)) => (None, tr("Next Departure").into(), n.clone()),
            (None, None) => (None, tr("No departures").into(), tr("Next 4 hours").into()),
        }
    }
}

impl Widget for DeparturesWidget {
    /// The minimum; the pill is as wide as its tile and `text_width`.
    fn size(&self) -> (f32, f32) {
        (48.0, 48.0)
    }

    fn refresh(&mut self) {
        self.refresh.request();
        self.notice = Some(tr("Refresh requested…").into());
    }

    fn tick(&mut self) {
        while let Ok(update) = self.rx.try_recv() {
            match update {
                Update::Data(board) => {
                    self.board = Some(board);
                    self.fetched = Some(Instant::now());
                    self.notice = None;
                }
                // Keep the timetable: it is usually still right, and the
                // traffic level turns Unknown on its own when it gets old.
                Update::Failed(reason) => self.notice = Some(reason),
                Update::Unconfigured => {
                    self.board = None;
                    self.notice = Some(tr("Choose a stop in settings").into());
                }
            }
        }
    }

    fn values(&self) -> crate::theme::Values {
        let (mode, primary, secondary) = self.lines();
        let now = Utc::now();
        let next = self.next(now);
        [
            ("icon".into(), mode_icon(mode).key().into()),
            ("primary".into(), primary),
            ("secondary".into(), secondary),
            ("traffic".into(), self.traffic().label().into()),
            ("mode".into(), mode.map(Mode::label).unwrap_or("").into()),
            (
                "line".into(),
                next.map(|d| d.line.clone()).unwrap_or_default(),
            ),
            (
                "minutes".into(),
                next.map(|d| ((d.expected - now).num_seconds().max(0) / 60).to_string())
                    .unwrap_or_default(),
            ),
            (
                "destination".into(),
                next.map(|d| d.destination.clone()).unwrap_or_default(),
            ),
        ]
        .into_iter()
        .collect()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _icons: &mut IconCache) {
        let (mode, first, second) = self.lines();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
            crate::builtin_icons::tile(ui, mode_icon(mode));
            if self.notice.is_none() && mode.is_some() {
                let level = self.traffic();
                ui.vertical(|ui| {
                    ui.set_width(text_width(ui));
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
                    crate::widgets::marquee::line(
                        ui,
                        egui::RichText::new(&first).size(12.0),
                        "first",
                    );
                    // One label in two colors, so the line sits exactly where
                    // the other pills' second lines do.
                    let font = egui::FontId::proportional(11.0);
                    let mut job = egui::text::LayoutJob::default();
                    job.append(
                        tr("Traffic: "),
                        0.0,
                        egui::TextFormat::simple(font.clone(), ui.visuals().weak_text_color()),
                    );
                    job.append(
                        level.label(),
                        0.0,
                        egui::TextFormat::simple(font, level.color()),
                    );
                    ui.add(egui::Label::new(job).truncate());
                });
            } else {
                two_lines_fixed(ui, &first, &second, text_width(ui));
            }
        });
    }
}

#[cfg(test)]
impl DeparturesWidget {
    /// Fixed sample data for the README banner (`single.rs`, `readme_banner`):
    /// no thread, no network, nothing from the user's own accounts.
    pub fn sample() -> Self {
        let at = Utc::now() + chrono::Duration::seconds(6 * 60 + 30);
        let journey = "sample".to_string();
        Self {
            refresh: Default::default(),
            rx: mpsc::channel().1,
            board: Some(Board {
                stop: "Sample stop".into(),
                departures: vec![Departure {
                    mode: Mode::Bus,
                    line: "31".into(),
                    destination: "Centre".into(),
                    aimed: at,
                    expected: at,
                    realtime: true,
                    cancelled: false,
                    journey: journey.clone(),
                    disruption: None,
                }],
                traffic: Level::Low,
                traffic_reason: String::new(),
                road: Some(RoadReading {
                    journey,
                    level: Level::Low,
                    reason: String::new(),
                    at: Instant::now(),
                }),
            }),
            fetched: Some(Instant::now()),
            notice: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws every mode's tile and the two-line layout; with
    /// `BTW_TILE_SHOT=<png>` the result is saved for a visual check.
    #[test]
    fn mode_tiles_render() {
        use crate::cpu_render::{self, Canvas, Textures};
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let (w, h) = (220u32, 34u32);
        let mut output = None;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(w as f32, h as f32),
                )),
                ..Default::default()
            };
            if let Some(previous) = output.take() {
                let previous: egui::FullOutput = previous;
                textures.set(&previous.textures_delta);
                textures.free(previous.textures_delta);
            }
            output = Some(ctx.run_ui(input, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(0x20, 0x24, 0x2c))
                    .inner_margin(4)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for mode in [
                                Some(Mode::Bus),
                                Some(Mode::Tram),
                                Some(Mode::Metro),
                                Some(Mode::Train),
                                Some(Mode::Boat),
                                None,
                            ] {
                                crate::builtin_icons::tile(ui, mode_icon(mode));
                            }
                        });
                    });
            }));
        }
        let output = output.unwrap();
        textures.set(&output.textures_delta);
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(!primitives.is_empty());
        if let Some(path) = std::env::var_os("BTW_TILE_SHOT") {
            let scale = 4;
            let mut canvas = Canvas::new((w * scale) as usize, (h * scale) as usize);
            cpu_render::paint(&mut canvas, &primitives, &textures, scale as f32);
            let pixels: Vec<u8> = canvas.pixels.iter().flat_map(|p| *p).collect();
            image::save_buffer(path, &pixels, w * scale, h * scale, image::ColorType::Rgba8)
                .unwrap();
        }
        textures.free(output.textures_delta);
    }

    fn cfg() -> DeparturesCfg {
        DeparturesCfg {
            stop: Some(Stop {
                id: "NSR:StopPlace:1".into(),
                name: "Stop".into(),
                lat: 59.9,
                lon: 10.7,
            }),
            ..Default::default()
        }
    }

    fn board_json() -> Value {
        json!({"data":{"stopPlace":{"name":"Stop","estimatedCalls":[
            {"realtime":true,"cancellation":false,
             "aimedDepartureTime":"2026-11-14T22:41:00+01:00","expectedDepartureTime":"2026-11-14T22:45:30+01:00",
             "destinationDisplay":{"frontText":"Bergkrystallen"},
             "serviceJourney":{"id":"RUT:ServiceJourney:a","line":{"publicCode":"4","transportMode":"metro",
                "situations":[{"severity":"severe","validityPeriod":{"startTime":"2026-11-19T00:00:00+01:00","endTime":null}}]}},
             "situations":[{"severity":"normal","validityPeriod":{"startTime":"2026-11-14T00:00:00+01:00","endTime":"2026-11-15T00:00:00+01:00"}}]},
            {"realtime":false,"cancellation":false,
             "aimedDepartureTime":"2026-11-14T22:38:00+01:00","expectedDepartureTime":"2026-11-14T22:38:00+01:00",
             "destinationDisplay":{"frontText":"Helsfyr"},
             "serviceJourney":{"id":"RUT:ServiceJourney:b","line":{"publicCode":"37","transportMode":"bus","situations":[]}},
             "situations":[]}
        ]}}})
    }

    #[test]
    fn departures_are_filtered_sorted_and_only_current_disruptions_count() {
        let now = DateTime::parse_from_rfc3339("2026-11-14T22:30:00+01:00")
            .unwrap()
            .to_utc();
        let all = parse_departures(&board_json(), &cfg(), now);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].line, "37");
        assert_eq!(all[0].mode, Mode::Bus);
        assert_eq!(all[0].disruption, None);
        // The line's closure next week does not count; today's message does.
        assert_eq!(all[1].disruption, Some(Level::Medium));
        assert_eq!(all[1].delay_minutes(), 4);
        let mut only = cfg();
        only.lines = vec![" 4 ".into()];
        only.direction = "berg".into();
        let filtered = parse_departures(&board_json(), &only, now);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].mode, Mode::Metro);
        only.direction = "Helsfyr".into();
        assert!(parse_departures(&board_json(), &only, now).is_empty());
        // A line that ends at several stops in one direction: any of them.
        only.direction = "Helsfyr, berg ,".into();
        assert_eq!(parse_departures(&board_json(), &only, now).len(), 1);
    }

    #[test]
    fn line_one_reads_like_the_spec() {
        let now = DateTime::parse_from_rfc3339("2026-11-14T22:30:00+01:00")
            .unwrap()
            .to_utc();
        let mut d = parse_departures(&board_json(), &cfg(), now).remove(0);
        d.line = "31".into();
        d.expected = now + chrono::Duration::seconds(6 * 60 + 20);
        d.aimed = d.expected;
        assert_eq!(headline(&d, now), "31 · 6 min");
        d.aimed = d.expected - chrono::Duration::minutes(3);
        assert_eq!(headline(&d, now), "31 · 6 min +3");
        d.expected = now + chrono::Duration::seconds(20);
        d.aimed = d.expected;
        assert_eq!(headline(&d, now), "31 · now");
        d.cancelled = true;
        assert_eq!(headline(&d, now), "31 · cancelled");
        // Without a line number the mode names the departure.
        d.line.clear();
        assert_eq!(headline(&d, now), "Bus · cancelled");
    }

    #[test]
    fn road_data_counts_only_for_its_own_journey_and_while_fresh() {
        let now = DateTime::parse_from_rfc3339("2026-11-14T22:30:00+01:00")
            .unwrap()
            .to_utc();
        let departures = parse_departures(&board_json(), &cfg(), now);
        let bus = departures[0].clone();
        let mut board = Board {
            stop: "Stop".into(),
            departures: departures.clone(),
            traffic: Level::None,
            traffic_reason: String::new(),
            road: Some(RoadReading {
                journey: bus.journey.clone(),
                level: Level::None,
                reason: String::new(),
                at: Instant::now(),
            }),
        };
        assert_eq!(board.traffic_for(&bus), Level::None);
        // The next bus on another journey has no reading of its own.
        let mut later = bus.clone();
        later.journey = "RUT:ServiceJourney:other".into();
        assert_eq!(board.traffic_for(&later), Level::Unknown);
        // A reading older than the limit is not trusted.
        if let Some(old) = Instant::now().checked_sub(ROAD_STALE + Duration::from_secs(1)) {
            board.road.as_mut().unwrap().at = old;
            assert_eq!(board.traffic_for(&bus), Level::Unknown);
        }
    }

    #[test]
    fn traffic_is_never_none_without_road_data() {
        assert_eq!(combine(Mode::Bus, None, None), Level::Unknown);
        assert_eq!(combine(Mode::Bus, None, Some(Level::None)), Level::Unknown);
        assert_eq!(combine(Mode::Bus, None, Some(Level::Medium)), Level::Medium);
        assert_eq!(combine(Mode::Bus, Some(Level::None), None), Level::None);
        assert_eq!(
            combine(Mode::Bus, Some(Level::Low), Some(Level::High)),
            Level::High
        );
        assert_eq!(combine(Mode::Train, None, None), Level::None);
        assert_eq!(combine(Mode::Metro, None, Some(Level::Low)), Level::Low);
    }

    #[test]
    fn only_road_situations_near_the_route_count() {
        // A straight route north along 10.75 E.
        let route = vec![(59.90, 10.75), (59.91, 10.75), (59.92, 10.75)];
        let feature = |lon: f64, lat: f64, kind: &str, sev: &str, periods: u64| {
            json!({"type":"Feature","geometry":{"type":"Point","coordinates":[lon,lat]},
                   "properties":{"ACTIVE":1,"SITUATION_TYPE":kind,"SEVERITY":sev,"NUM_PERIODS":periods}})
        };
        // ~110 m east of the route: counts. ~2 km east: does not.
        let near = feature(10.752, 59.905, "Accident", "none", 0);
        let far = feature(10.79, 59.905, "Accident", "highest", 0);
        let geo = json!({"features":[near, far]});
        assert_eq!(road_traffic(&geo, &route, 300.0).0, Level::Medium);
        let geo = json!({"features":[feature(10.751, 59.915, "MaintenanceWorks", "low", 3)]});
        assert_eq!(road_traffic(&geo, &route, 300.0).0, Level::Low);
        assert_eq!(
            road_traffic(&json!({"features":[]}), &route, 300.0).0,
            Level::None
        );
        // A road closure line crossing the route, with no vertex near it.
        let crossing = json!({"features":[{"geometry":{"type":"LineString","coordinates":[[10.70,59.915],[10.80,59.915]]},
            "properties":{"ACTIVE":1,"SITUATION_TYPE":"RoadOrCarriagewayOrLaneManagement","SEVERITY":"low",
                          "ROAD_OR_CARRIAGEWAY_OR_LANE_MANAGEMENT_TYPE":"roadClosed","NUM_PERIODS":0}}]});
        assert_eq!(road_traffic(&crossing, &route, 300.0).0, Level::High);
    }

    #[test]
    fn route_starts_at_our_stop() {
        let journey = json!({"data":{"serviceJourney":{"quays":[
            {"stopPlace":{"id":"NSR:StopPlace:0"},"latitude":1.0,"longitude":1.0},
            {"stopPlace":{"id":"NSR:StopPlace:1"},"latitude":2.0,"longitude":2.0},
            {"stopPlace":{"id":"NSR:StopPlace:2"},"latitude":3.0,"longitude":3.0}]}}});
        assert_eq!(
            route_from(&journey, "NSR:StopPlace:1"),
            vec![(2.0, 2.0), (3.0, 3.0)]
        );
    }

    #[test]
    fn stop_search_and_ids_are_checked() {
        let found = parse_stops(&json!({"features":[
            {"properties":{"id":"NSR:StopPlace:58366","label":"Jernbanetorget, Oslo"},"geometry":{"coordinates":[10.75,59.91]}},
            {"properties":{"id":"KVE:TopographicPlace:1","label":"Oslo"},"geometry":{"coordinates":[10.75,59.91]}}
        ]}));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].lat, 59.91);
        assert!(!valid_id("NSR:StopPlace:1\") { x }"));
        assert!(valid_id("RUT:ServiceJourney:7a4ceacb"));
    }
}
