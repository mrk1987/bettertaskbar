//! iCalendar (RFC 5545) reading for the calendar pill: unfold lines, read the
//! VEVENTs, resolve time zones, and expand recurring events into the concrete
//! occurrences inside a time window.
//!
//! All-day events are left out on purpose: the pill shows what is on now and
//! what starts next, and an all-day entry would sit in "Now" the whole day.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub summary: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

#[derive(Debug, Default)]
pub struct Expanded {
    /// Occurrences overlapping the window, sorted by start.
    pub events: Vec<Event>,
    /// Recurrence rules that could not be read; those events count once, at
    /// their first start.
    pub bad_rules: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum When {
    Timed(DateTime<Utc>),
    AllDay,
}

#[derive(Debug, Default)]
struct RawEvent {
    uid: String,
    summary: String,
    start: Option<When>,
    end: Option<When>,
    duration: Option<Duration>,
    /// The DTSTART line as written, for the recurrence parser.
    dtstart_line: String,
    /// RRULE / RDATE / EXDATE lines as written.
    rule_lines: Vec<String>,
    recurrence_id: Option<When>,
    cancelled: bool,
}

/// One content line split into name, parameters and value.
struct Prop<'a> {
    name: &'a str,
    params: Vec<(&'a str, &'a str)>,
    value: &'a str,
}

impl<'a> Prop<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        // The value starts at the first ':' outside a quoted parameter value.
        let mut quoted = false;
        let colon = line.char_indices().find_map(|(i, c)| match c {
            '"' => {
                quoted = !quoted;
                None
            }
            ':' if !quoted => Some(i),
            _ => None,
        })?;
        let (head, value) = (&line[..colon], &line[colon + 1..]);
        let mut parts = split_unquoted(head, ';').into_iter();
        let name = parts.next()?;
        let params = parts
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k, v.trim_matches('"')))
            .collect();
        Some(Self {
            name,
            params,
            value,
        })
    }

    fn param(&self, key: &str) -> Option<&'a str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| *v)
    }
}

fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == sep && !quoted {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

/// Undo line folding: a line break followed by a space or tab continues the
/// previous line.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (
            raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')),
            lines.last_mut(),
        ) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(raw.to_owned()),
        }
    }
    lines
}

fn parse_when(prop: &Prop) -> Option<When> {
    let v = prop.value.trim();
    if prop
        .param("VALUE")
        .is_some_and(|x| x.eq_ignore_ascii_case("DATE"))
        || v.len() == 8
    {
        NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        return Some(When::AllDay);
    }
    if let Some(utc) = v.strip_suffix('Z') {
        let n = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some(When::Timed(Utc.from_utc_datetime(&n)));
    }
    let n = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    // A TZID this build does not know (or none: "floating" time) is read as
    // local time, which is right for the common case of the user's own zone.
    let utc = match prop
        .param("TZID")
        .and_then(|tz| tz.parse::<chrono_tz::Tz>().ok())
    {
        Some(tz) => resolve(&tz, n)?.with_timezone(&Utc),
        None => resolve(&Local, n)?.with_timezone(&Utc),
    };
    Some(When::Timed(utc))
}

/// Wall-clock time to an instant. A time skipped by a DST jump is moved an
/// hour later; an ambiguous one takes the first occurrence.
fn resolve<Z: TimeZone>(tz: &Z, n: NaiveDateTime) -> Option<DateTime<Z>> {
    tz.from_local_datetime(&n)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(n + Duration::hours(1))).earliest())
}

/// RFC 5545 DURATION, e.g. `PT1H30M`, `P1D`, `P2W`, `-PT15M`.
fn parse_duration(v: &str) -> Option<Duration> {
    let v = v.trim();
    let (sign, rest) = match v.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, v.strip_prefix('+').unwrap_or(v)),
    };
    let rest = rest.strip_prefix('P')?;
    let mut secs: i64 = 0;
    let mut num = String::new();
    for c in rest.chars() {
        match c {
            '0'..='9' => num.push(c),
            'T' => {}
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                let multiplier = match unit {
                    'W' => 604_800,
                    'D' => 86_400,
                    'H' => 3_600,
                    'M' => 60,
                    'S' => 1,
                    _ => return None,
                };
                secs = secs.checked_add(n.checked_mul(multiplier)?)?;
            }
        }
    }
    if !num.is_empty() {
        return None;
    }
    Duration::try_seconds(sign * secs)
}

fn unescape_text(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push(' '),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out.trim().to_owned()
}

fn parse_events(text: &str, out: &mut Vec<RawEvent>) {
    let mut current: Option<RawEvent> = None;
    // Depth of components nested inside the VEVENT (VALARM); their properties
    // (a VALARM has its own DESCRIPTION/DURATION) are not the event's.
    let mut nested = 0usize;
    for line in unfold(text) {
        let Some(prop) = Prop::parse(&line) else {
            continue;
        };
        let name = prop.name.to_ascii_uppercase();
        match (name.as_str(), current.as_mut()) {
            ("BEGIN", None) if prop.value.eq_ignore_ascii_case("VEVENT") => {
                current = Some(RawEvent::default())
            }
            ("BEGIN", Some(_)) => nested += 1,
            ("END", Some(_)) if nested > 0 => nested -= 1,
            ("END", Some(_)) if prop.value.eq_ignore_ascii_case("VEVENT") => {
                out.extend(current.take())
            }
            (_, Some(_)) if nested > 0 => {}
            ("UID", Some(ev)) => ev.uid = prop.value.trim().to_owned(),
            ("SUMMARY", Some(ev)) => ev.summary = unescape_text(prop.value),
            ("DTSTART", Some(ev)) => {
                ev.start = parse_when(&prop);
                ev.dtstart_line = line.clone();
            }
            ("DTEND", Some(ev)) => ev.end = parse_when(&prop),
            ("DURATION", Some(ev)) => ev.duration = parse_duration(prop.value),
            ("RECURRENCE-ID", Some(ev)) => ev.recurrence_id = parse_when(&prop),
            ("STATUS", Some(ev)) => {
                ev.cancelled = prop.value.trim().eq_ignore_ascii_case("CANCELLED")
            }
            ("RRULE" | "RDATE" | "EXDATE", Some(ev)) => ev.rule_lines.push(line.clone()),
            _ => {}
        }
    }
}

/// Every timed occurrence that overlaps `[from, to)`, from any number of
/// iCalendar documents (one per calendar object resource).
pub fn occurrences<'a>(
    docs: impl IntoIterator<Item = &'a str>,
    from: DateTime<Local>,
    to: DateTime<Local>,
) -> Expanded {
    let mut raw = Vec::new();
    for doc in docs {
        parse_events(doc, &mut raw);
    }
    let from_utc = from.with_timezone(&Utc);
    let to_utc = to.with_timezone(&Utc);

    // An event with RECURRENCE-ID replaces that one occurrence of its series,
    // also when it is cancelled.
    let overridden: HashSet<(&str, DateTime<Utc>)> = raw
        .iter()
        .filter_map(|e| match e.recurrence_id {
            Some(When::Timed(t)) => Some((e.uid.as_str(), t)),
            _ => None,
        })
        .collect();

    let mut result = Expanded::default();
    for ev in &raw {
        let Some(When::Timed(start)) = ev.start else {
            continue;
        };
        if ev.cancelled {
            continue;
        }
        let length = match (ev.end, ev.duration) {
            (Some(When::Timed(end)), _) if end > start => end - start,
            (_, Some(d)) if d > Duration::zero() => d,
            _ => Duration::zero(),
        };
        let mut starts = vec![start];
        if ev.recurrence_id.is_none() && !ev.rule_lines.is_empty() {
            let Some(after) = from_utc.checked_sub_signed(length) else {
                continue;
            };
            match expand(ev, after, to_utc) {
                Some(dates) => {
                    starts = dates;
                    starts.retain(|s| !overridden.contains(&(ev.uid.as_str(), *s)));
                }
                None => result.bad_rules += 1,
            }
        }
        for s in starts {
            let Some(e) = s.checked_add_signed(length) else {
                continue;
            };
            // A zero-length event counts at its start instant.
            if s < to_utc && (e > from_utc || s >= from_utc) {
                result.events.push(Event {
                    summary: if ev.summary.is_empty() {
                        "(busy)".to_owned()
                    } else {
                        ev.summary.clone()
                    },
                    start: s.with_timezone(&Local),
                    end: e.with_timezone(&Local),
                });
            }
        }
    }
    result.events.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.summary.cmp(&b.summary))
    });
    result
}

/// Occurrence starts of a recurring event between `after` and `before`.
fn expand(
    ev: &RawEvent,
    after: DateTime<Utc>,
    before: DateTime<Utc>,
) -> Option<Vec<DateTime<Utc>>> {
    let text = format!("{}\n{}", ev.dtstart_line, ev.rule_lines.join("\n"));
    let set: rrule::RRuleSet = text.parse().ok()?;
    let utc = rrule::Tz::UTC;
    // One second of slack on both sides; the caller filters exactly.
    let found = set
        .after((after - Duration::seconds(1)).with_timezone(&utc))
        .before((before + Duration::seconds(1)).with_timezone(&utc))
        .all(2000);
    Some(
        found
            .dates
            .into_iter()
            .map(|d| d.with_timezone(&Utc))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .with_timezone(&Local)
    }

    fn cal(body: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}END:VCALENDAR\r\n",
            body.replace('\n', "\r\n")
        )
    }

    fn starts(x: &Expanded) -> Vec<DateTime<Utc>> {
        x.events
            .iter()
            .map(|e| e.start.with_timezone(&Utc))
            .collect()
    }

    #[test]
    fn extreme_durations_do_not_panic_or_wrap() {
        for duration in [
            "P9223372036854775807D",
            "PT9223372036854775807S",
            "P1000000000000D",
        ] {
            assert!(parse_duration(duration).is_none(), "{duration}");
        }
        let doc = cal(
            "BEGIN:VEVENT\nUID:huge\nDTSTART:20261116T100000Z\nDURATION:P100000000D\nEND:VEVENT\n",
        );
        let expanded = occurrences(
            [doc.as_str()],
            utc(2026, 11, 16, 0, 0),
            utc(2026, 11, 17, 0, 0),
        );
        assert!(expanded.events.is_empty());
    }

    #[test]
    fn tzid_is_converted_not_read_as_local() {
        // 09:00 in London in November is 09:00 UTC; in Nicosia it is 07:00 UTC.
        let doc = cal("BEGIN:VEVENT\nUID:a\nSUMMARY:London\nDTSTART;TZID=Europe/London:20261116T090000\nDTEND;TZID=Europe/London:20261116T100000\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:b\nSUMMARY:Nicosia\nDTSTART;TZID=Asia/Nicosia:20261116T090000\nDTEND;TZID=Asia/Nicosia:20261116T093000\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 16, 0, 0),
            utc(2026, 11, 17, 0, 0),
        );
        assert_eq!(x.events.len(), 2);
        assert_eq!(x.events[0].summary, "Nicosia");
        assert_eq!(x.events[0].start, utc(2026, 11, 16, 7, 0));
        assert_eq!(x.events[0].end, utc(2026, 11, 16, 7, 30));
        assert_eq!(x.events[1].start, utc(2026, 11, 16, 9, 0));
    }

    #[test]
    fn utc_and_window_filter() {
        let doc = cal("BEGIN:VEVENT\nUID:old\nSUMMARY:Old\nDTSTART:20240101T100000Z\nDTEND:20240101T110000Z\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:in\nSUMMARY:In\nDTSTART:20261116T100000Z\nDTEND:20261116T110000Z\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:late\nSUMMARY:Late\nDTSTART:20270101T100000Z\nDTEND:20270101T110000Z\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 16, 10, 30),
            utc(2026, 11, 30, 0, 0),
        );
        assert_eq!(x.events.len(), 1);
        assert_eq!(x.events[0].summary, "In");
    }

    #[test]
    fn all_day_and_cancelled_are_skipped() {
        let doc = cal("BEGIN:VEVENT\nUID:d\nSUMMARY:Birthday\nDTSTART;VALUE=DATE:20261116\nDTEND;VALUE=DATE:20261117\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:c\nSUMMARY:Off\nSTATUS:CANCELLED\nDTSTART:20261116T100000Z\nDTEND:20261116T110000Z\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 16, 0, 0),
            utc(2026, 11, 17, 0, 0),
        );
        assert!(x.events.is_empty());
    }

    #[test]
    fn recurring_event_from_years_ago_shows_this_weeks_occurrence() {
        // Monthly on the 16th since 2022, 10:00 Oslo time (09:00 UTC in winter).
        let doc = cal("BEGIN:VEVENT\nUID:m\nSUMMARY:Monthly\nDTSTART;TZID=Europe/Oslo:20220116T100000\nDTEND;TZID=Europe/Oslo:20220116T110000\nRRULE:FREQ=MONTHLY\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 10, 0, 0),
            utc(2026, 12, 20, 0, 0),
        );
        assert_eq!(x.bad_rules, 0);
        assert_eq!(
            starts(&x),
            vec![utc(2026, 11, 16, 9, 0), utc(2026, 12, 16, 9, 0)]
        );
        assert_eq!(x.events[0].end, utc(2026, 11, 16, 10, 0));
    }

    #[test]
    fn occurrence_already_running_at_window_start_is_included() {
        let doc = cal("BEGIN:VEVENT\nUID:d\nSUMMARY:Daily\nDTSTART:20260101T090000Z\nDTEND:20260101T100000Z\nRRULE:FREQ=DAILY\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 16, 9, 30),
            utc(2026, 11, 17, 9, 0),
        );
        // 16th 09:00 (running) only; the 17th 09:00 starts at the window end.
        assert_eq!(starts(&x), vec![utc(2026, 11, 16, 9, 0)]);
    }

    #[test]
    fn until_count_and_exdate() {
        let doc = cal("BEGIN:VEVENT\nUID:u\nSUMMARY:Ended\nDTSTART:20260101T090000Z\nDTEND:20260101T100000Z\nRRULE:FREQ=DAILY;UNTIL=20260201T000000Z\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:c\nSUMMARY:Three\nDTSTART:20261115T090000Z\nDTEND:20261115T100000Z\nRRULE:FREQ=DAILY;COUNT=3\nEXDATE:20261116T090000Z\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 1, 0, 0),
            utc(2026, 12, 1, 0, 0),
        );
        assert_eq!(x.bad_rules, 0);
        assert_eq!(
            starts(&x),
            vec![utc(2026, 11, 15, 9, 0), utc(2026, 11, 17, 9, 0)]
        );
    }

    #[test]
    fn override_replaces_its_occurrence() {
        // Weekly Monday 09:00 UTC; the 16 Nov instance is moved to 14:00 and
        // the 23 Nov instance is cancelled.
        let doc = cal("BEGIN:VEVENT\nUID:w\nSUMMARY:Weekly\nDTSTART:20261102T090000Z\nDTEND:20261102T100000Z\nRRULE:FREQ=WEEKLY\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:w\nSUMMARY:Weekly (moved)\nRECURRENCE-ID:20261116T090000Z\nDTSTART:20261116T140000Z\nDTEND:20261116T150000Z\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:w\nSUMMARY:Weekly\nSTATUS:CANCELLED\nRECURRENCE-ID:20261123T090000Z\nDTSTART:20261123T090000Z\nDTEND:20261123T100000Z\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 9, 0, 0),
            utc(2026, 12, 1, 0, 0),
        );
        assert_eq!(
            starts(&x),
            vec![
                utc(2026, 11, 9, 9, 0),
                utc(2026, 11, 16, 14, 0),
                utc(2026, 11, 30, 9, 0)
            ]
        );
        assert_eq!(x.events[1].summary, "Weekly (moved)");
    }

    #[test]
    fn yearly_bymonth_with_until_like_the_bridge_sends() {
        let doc = cal("BEGIN:VEVENT\nUID:y\nSUMMARY:Yearly\nDTSTART;TZID=Europe/Berlin:20251120T180000\nDTEND;TZID=Europe/Berlin:20251120T190000\nRRULE:FREQ=YEARLY;UNTIL=20301231T225959Z;INTERVAL=1;BYMONTHDAY=20;BYMONTH=11\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 1, 0, 0),
            utc(2026, 12, 1, 0, 0),
        );
        assert_eq!(x.bad_rules, 0);
        assert_eq!(starts(&x), vec![utc(2026, 11, 20, 17, 0)]);
    }

    #[test]
    fn folding_valarm_and_escapes() {
        let doc = cal("BEGIN:VEVENT\nUID:f\nSUMMARY:Lunch\\, with a very long\n  title\\; really\nDTSTART:20261116T110000Z\nDURATION:PT45M\nBEGIN:VALARM\nACTION:DISPLAY\nDESCRIPTION:Reminder\nDURATION:PT5M\nTRIGGER:-PT15M\nEND:VALARM\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 16, 0, 0),
            utc(2026, 11, 17, 0, 0),
        );
        assert_eq!(x.events.len(), 1);
        assert_eq!(x.events[0].summary, "Lunch, with a very long title; really");
        assert_eq!(x.events[0].end, utc(2026, 11, 16, 11, 45));
    }

    #[test]
    fn unreadable_rule_falls_back_to_first_start() {
        let doc = cal("BEGIN:VEVENT\nUID:x\nSUMMARY:Odd\nDTSTART:20261116T110000Z\nDTEND:20261116T120000Z\nRRULE:FREQ=SOMETIMES\nEND:VEVENT\n");
        let x = occurrences(
            [doc.as_str()],
            utc(2026, 11, 16, 0, 0),
            utc(2026, 11, 17, 0, 0),
        );
        assert_eq!(x.bad_rules, 1);
        assert_eq!(x.events.len(), 1);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(parse_duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(parse_duration("P2W"), Some(Duration::weeks(2)));
        assert_eq!(parse_duration("1H"), None);
        assert_eq!(parse_duration("PT5"), None);
    }
}
