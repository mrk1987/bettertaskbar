//! CalDAV client for the calendar pill: find the calendars under the
//! configured path, fetch their events, and pick what is on now and what
//! comes next.
//!
//! Discovery follows RFC 4791/5397: the configured path may be a principal, a
//! calendar home or a single calendar. With the iOxide bridge the principal is
//! `/caldav/`, the home is `/caldav/calendars/` and each calendar is
//! `/caldav/calendars/<id>`; none of that is assumed here.

use base64::Engine;
use chrono::{DateTime, Duration, Local, Utc};

use crate::config::CalendarCfg;
use crate::ical::{self, Event};

const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
/// A bridge may ignore the time-range filter and send the whole calendar.
const MAX_BODY: u64 = 64 * 1024 * 1024;
const LOOKAHEAD_DAYS: i64 = 14;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lines {
    pub now: Option<String>,
    pub next: Option<String>,
}

#[derive(Debug)]
pub struct Fetched {
    pub lines: Lines,
    pub calendars: usize,
    pub events: usize,
    pub bad_rules: usize,
}

pub fn fetch(cfg: &CalendarCfg) -> Result<Fetched, String> {
    let client = Client::new(cfg);
    let calendars = client.discover()?;
    let now = Local::now();
    let until = now + Duration::days(LOOKAHEAD_DAYS);

    let mut docs: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for href in &calendars {
        match client.calendar_data(href, now, until) {
            Ok(mut d) => docs.append(&mut d),
            Err(e) => failures.push(e),
        }
    }
    // One broken calendar should not blank the pill, but all of them should.
    if failures.len() == calendars.len() {
        return Err(failures.remove(0));
    }
    let expanded = ical::occurrences(docs.iter().map(String::as_str), now, until);
    Ok(Fetched {
        lines: lines(&expanded.events, now),
        calendars: calendars.len(),
        events: expanded.events.len(),
        bad_rules: expanded.bad_rules,
    })
}

/// "Now" is the running event that ends first; "Next" the first one to start.
fn lines(events: &[Event], now: DateTime<Local>) -> Lines {
    let current = events
        .iter()
        .filter(|e| e.start <= now && now < e.end)
        .min_by_key(|e| e.end);
    let next = events.iter().find(|e| e.start > now);
    Lines {
        now: current.map(|e| format!("Now: {} · to {}", e.summary, e.end.format("%H:%M"))),
        next: next.map(|e| format!("Next: {} · {}", e.summary, when(e.start, now))),
    }
}

/// Start time for the pill: the clock time today, with the weekday or date
/// added when the event is on another day.
fn when(start: DateTime<Local>, now: DateTime<Local>) -> String {
    let days = (start.date_naive() - now.date_naive()).num_days();
    match days {
        0 => start.format("%H:%M").to_string(),
        1..=6 => start.format("%a %H:%M").to_string(),
        _ => start.format("%-d %b %H:%M").to_string(),
    }
}

struct Client<'a> {
    cfg: &'a CalendarCfg,
    agent: ureq::Agent,
    auth: String,
}

/// One `<response>` of a multistatus, reduced to what discovery needs.
#[derive(Debug, Default, PartialEq)]
struct Resource {
    href: String,
    is_calendar: bool,
    home_set: Option<String>,
    principal: Option<String>,
}

impl<'a> Client<'a> {
    fn new(cfg: &'a CalendarCfg) -> Self {
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", cfg.username, cfg.password));
        Self {
            cfg,
            agent: crate::http::webdav_agent(&cfg.host),
            auth: format!("Basic {token}"),
        }
    }

    fn url(&self, path: &str) -> Result<String, String> {
        let scheme = if self.cfg.tls { "https" } else { "http" };
        let host = if self.cfg.host.contains(':') && !self.cfg.host.starts_with('[') {
            format!("[{}]", self.cfg.host)
        } else {
            self.cfg.host.clone()
        };
        let origin = format!("{scheme}://{host}:{}", self.cfg.port);
        let base: ureq::http::Uri = origin.parse().map_err(|_| "invalid calendar host")?;
        if path.starts_with("http://") || path.starts_with("https://") {
            let uri: ureq::http::Uri = path.parse().map_err(|_| "invalid calendar URL")?;
            let port = |uri: &ureq::http::Uri| {
                uri.port_u16()
                    .unwrap_or(if uri.scheme_str() == Some("https") {
                        443
                    } else {
                        80
                    })
            };
            if uri.scheme() != base.scheme()
                || !uri
                    .host()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(base.host().unwrap_or(""))
                || port(&uri) != port(&base)
                || uri.authority().is_some_and(|a| a.as_str().contains('@'))
            {
                return Err("calendar server returned a URL outside its configured origin".into());
            }
            return Ok(path.to_owned());
        }
        let slash = if path.starts_with('/') { "" } else { "/" };
        Ok(format!("{origin}{slash}{path}"))
    }

    /// Send a WebDAV request and return the 207 body. Any other outcome is an
    /// error that says what was asked and what came back.
    fn dav(&self, method: &str, path: &str, depth: &str, body: &str) -> Result<String, String> {
        let request = ureq::http::Request::builder()
            .method(method)
            .uri(self.url(path)?)
            .header("Authorization", &self.auth)
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body.to_owned())
            .map_err(|e| format!("{method} {path}: bad request: {e}"))?;
        let mut response = self
            .agent
            .run(request)
            .map_err(|e| format!("{method} {path}: {e}"))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(|e| format!("{method} {path}: reading the reply failed: {e}"))?;
        match status {
            207 => Ok(text),
            401 | 403 => Err(format!(
                "{method} {path}: HTTP {status}, check username and password"
            )),
            _ => {
                let detail: String = text.trim().chars().take(120).collect();
                Err(format!("{method} {path}: HTTP {status} {detail}"))
            }
        }
    }

    fn propfind(&self, path: &str, depth: &str) -> Result<Vec<Resource>, String> {
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
            <D:propfind xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\"><D:prop>\
            <D:resourcetype/><D:current-user-principal/><C:calendar-home-set/>\
            </D:prop></D:propfind>";
        let xml = self.dav("PROPFIND", path, depth, body)?;
        parse_resources(&xml).map_err(|e| format!("PROPFIND {path}: {e}"))
    }

    /// Paths of the calendars reachable from the configured path.
    fn discover(&self) -> Result<Vec<String>, String> {
        let configured = if self.cfg.path.starts_with('/') {
            self.cfg.path.clone()
        } else {
            format!("/{}", self.cfg.path)
        };
        // Servers disagree on the trailing slash: iOxide answers `/caldav` with
        // an empty multistatus and only `/caldav/` with the principal.
        let mut starts = vec![configured.clone()];
        if !configured.ends_with('/') {
            starts.push(format!("{configured}/"));
        }

        let mut last_error = format!("PROPFIND {configured}: the server listed nothing there");
        for start in &starts {
            let resource = match self.propfind(start, "0") {
                Ok(list) => match list.into_iter().next() {
                    Some(r) => r,
                    None => continue,
                },
                Err(e) => {
                    last_error = e;
                    continue;
                }
            };
            if resource.is_calendar {
                return Ok(vec![start.clone()]);
            }
            let mut home = resource.home_set.clone();
            if home.is_none() {
                if let Some(principal) = resource.principal.filter(|p| p != start) {
                    home = self
                        .propfind(&principal, "0")
                        .ok()
                        .and_then(|l| l.into_iter().find_map(|r| r.home_set));
                }
            }
            let home = home.unwrap_or_else(|| start.clone());
            match self.propfind(&home, "1") {
                Ok(children) => {
                    let calendars: Vec<String> = children
                        .into_iter()
                        .filter(|r| r.is_calendar)
                        .map(|r| r.href)
                        .collect();
                    if !calendars.is_empty() {
                        return Ok(calendars);
                    }
                    last_error = format!("no calendars found under {home}");
                }
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    /// The iCalendar text of every event resource in a calendar. The time
    /// range is a hint; the caller filters, since a server may ignore it.
    fn calendar_data(
        &self,
        href: &str,
        from: DateTime<Local>,
        to: DateTime<Local>,
    ) -> Result<Vec<String>, String> {
        let stamp = |t: DateTime<Local>| t.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ").to_string();
        let query = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
             <C:calendar-query xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\">\
             <D:prop><C:calendar-data/></D:prop>\
             <C:filter><C:comp-filter name=\"VCALENDAR\"><C:comp-filter name=\"VEVENT\">\
             <C:time-range start=\"{}\" end=\"{}\"/>\
             </C:comp-filter></C:comp-filter></C:filter></C:calendar-query>",
            stamp(from),
            stamp(to)
        );
        let xml = self.dav("REPORT", href, "1", &query)?;
        parse_calendar_data(&xml).map_err(|e| format!("REPORT {href}: {e}"))
    }
}

fn parse_xml(xml: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse(xml).map_err(|e| format!("the reply is not valid XML: {e}"))
}

fn is(node: &roxmltree::Node, ns: &str, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(ns)
}

fn find<'a, 'i>(
    node: roxmltree::Node<'a, 'i>,
    ns: &str,
    name: &str,
) -> Option<roxmltree::Node<'a, 'i>> {
    node.descendants().find(|n| is(n, ns, name))
}

fn href_in(node: roxmltree::Node) -> Option<String> {
    find(node, DAV, "href")
        .and_then(|h| h.text())
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
}

fn parse_resources(xml: &str) -> Result<Vec<Resource>, String> {
    let doc = parse_xml(xml)?;
    let mut out = Vec::new();
    for response in doc.descendants().filter(|n| is(n, DAV, "response")) {
        let Some(href) = response
            .children()
            .find(|n| is(n, DAV, "href"))
            .and_then(|h| h.text())
        else {
            continue;
        };
        let mut resource = Resource {
            href: href.trim().to_owned(),
            ..Default::default()
        };
        // Only properties the server actually has: a 404 propstat lists the
        // ones it does not, as empty elements.
        for propstat in response.children().filter(|n| is(n, DAV, "propstat")) {
            let ok = find(propstat, DAV, "status")
                .and_then(|s| s.text())
                .is_none_or(|s| s.contains(" 200"));
            if !ok {
                continue;
            }
            if let Some(kind) = find(propstat, DAV, "resourcetype") {
                resource.is_calendar |= kind.children().any(|n| is(&n, CALDAV, "calendar"));
            }
            if let Some(h) = find(propstat, CALDAV, "calendar-home-set").and_then(href_in) {
                resource.home_set = Some(h);
            }
            if let Some(h) = find(propstat, DAV, "current-user-principal").and_then(href_in) {
                resource.principal = Some(h);
            }
        }
        out.push(resource);
    }
    Ok(out)
}

fn parse_calendar_data(xml: &str) -> Result<Vec<String>, String> {
    let doc = parse_xml(xml)?;
    Ok(doc
        .descendants()
        .filter(|n| is(n, CALDAV, "calendar-data"))
        .map(|n| n.children().filter_map(|c| c.text()).collect::<String>())
        .filter(|t| !t.trim().is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn calendar_urls_never_forward_credentials_to_another_origin() {
        let cfg = CalendarCfg {
            host: "calendar.example".into(),
            port: 443,
            tls: true,
            ..Default::default()
        };
        let client = Client::new(&cfg);
        assert_eq!(
            client.url("/home/").unwrap(),
            "https://calendar.example:443/home/"
        );
        assert!(client.url("https://CALENDAR.example/home/").is_ok());
        for url in [
            "https://other.example/home/",
            "http://calendar.example/home/",
            "https://calendar.example:444/home/",
            "https://user@calendar.example/home/",
        ] {
            assert!(client.url(url).is_err(), "{url}");
        }
        let cfg = CalendarCfg {
            host: "::1".into(),
            ..Default::default()
        };
        assert_eq!(
            Client::new(&cfg).url("caldav/").unwrap(),
            "http://[::1]:8081/caldav/"
        );
    }

    // Shapes copied from the iOxide bridge (go-webdav): default namespaces on
    // every element, a 404 propstat with empty placeholders.
    const PRINCIPAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<multistatus xmlns="DAV:"><response xmlns="DAV:"><href>/caldav/</href><propstat xmlns="DAV:"><prop xmlns="DAV:"><resourcetype xmlns="DAV:"><collection xmlns="DAV:"></collection><principal xmlns="DAV:"></principal></resourcetype><current-user-principal xmlns="DAV:"><href>/caldav/</href></current-user-principal><calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:">/caldav/calendars/</href></calendar-home-set></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;

    const HOME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<multistatus xmlns="DAV:"><response xmlns="DAV:"><href>/caldav/calendars/</href><propstat xmlns="DAV:"><prop xmlns="DAV:"><resourcetype xmlns="DAV:"><collection xmlns="DAV:"></collection></resourcetype><current-user-principal xmlns="DAV:"><href>/caldav/</href></current-user-principal></prop><status>HTTP/1.1 200 OK</status></propstat><propstat xmlns="DAV:"><prop xmlns="DAV:"><calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"></calendar-home-set></prop><status>HTTP/1.1 404 Not Found</status></propstat></response><response xmlns="DAV:"><href>/caldav/calendars/abc==</href><propstat xmlns="DAV:"><prop xmlns="DAV:"><resourcetype xmlns="DAV:"><collection xmlns="DAV:"></collection><calendar xmlns="urn:ietf:params:xml:ns:caldav"></calendar></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;

    #[test]
    fn principal_points_at_home_set() {
        let r = parse_resources(PRINCIPAL).unwrap();
        assert_eq!(r.len(), 1);
        assert!(!r[0].is_calendar);
        assert_eq!(r[0].home_set.as_deref(), Some("/caldav/calendars/"));
        assert_eq!(r[0].principal.as_deref(), Some("/caldav/"));
    }

    #[test]
    fn home_lists_calendars_and_ignores_404_props() {
        let r = parse_resources(HOME).unwrap();
        assert_eq!(r.len(), 2);
        assert!(!r[0].is_calendar);
        assert_eq!(r[0].home_set, None);
        assert!(r[1].is_calendar);
        assert_eq!(r[1].href, "/caldav/calendars/abc==");
    }

    #[test]
    fn empty_multistatus_is_no_resources() {
        let r = parse_resources(r#"<?xml version="1.0"?><multistatus xmlns="DAV:"></multistatus>"#)
            .unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn prefixed_namespaces_work_too() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/c/1/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><cal:calendar/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let r = parse_resources(xml).unwrap();
        assert!(r[0].is_calendar);
    }

    #[test]
    fn calendar_data_entities_are_decoded() {
        // The bridge escapes line breaks as &#xD;&#xA; and quotes as &#34;.
        let xml = r#"<multistatus xmlns="DAV:"><response><href>/x</href><propstat><prop><calendar-data xmlns="urn:ietf:params:xml:ns:caldav">BEGIN:VCALENDAR&#xD;&#xA;BEGIN:VEVENT&#xD;&#xA;SUMMARY:A &amp; B &#34;q&#34;&#xD;&#xA;END:VEVENT&#xD;&#xA;END:VCALENDAR&#xD;&#xA;</calendar-data></prop></propstat></response></multistatus>"#;
        let d = parse_calendar_data(xml).unwrap();
        assert_eq!(d.len(), 1);
        assert!(d[0].contains("\r\nSUMMARY:A & B \"q\"\r\n"));
    }

    fn ev(summary: &str, start: DateTime<Local>, mins: i64) -> Event {
        Event {
            summary: summary.into(),
            start,
            end: start + Duration::minutes(mins),
        }
    }

    #[test]
    fn now_and_next_lines() {
        let now = Local.with_ymd_and_hms(2026, 11, 16, 10, 30, 0).unwrap();
        let at = |d: u32, h: u32, m: u32| Local.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap();
        let events = vec![
            ev("Standup", at(16, 10, 0), 60),
            ev("Lunch", at(16, 12, 0), 30),
            ev("Review", at(17, 9, 0), 30),
        ];
        let l = lines(&events, now);
        assert_eq!(l.now.as_deref(), Some("Now: Standup · to 11:00"));
        assert_eq!(l.next.as_deref(), Some("Next: Lunch · 12:00"));

        let l = lines(&events[2..], now);
        assert_eq!(l.now, None);
        assert_eq!(l.next.as_deref(), Some("Next: Review · Tue 09:00"));

        let far = vec![ev("Trip", at(27, 7, 15), 30)];
        assert_eq!(
            lines(&far, now).next.as_deref(),
            Some("Next: Trip · 27 Nov 07:15")
        );
    }
}
