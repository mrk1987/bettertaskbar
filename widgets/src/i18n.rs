//! Interface language: English, the source language, and Norwegian bokmål.
//!
//! Text is written in English and wrapped in [`tr`] (or [`trf`] when it has
//! values in it); Norwegian comes from the `NB` table in `i18n_nb.rs`. A
//! test reads every literal passed to `tr` or `trf` and fails when the
//! table lacks one, has one nothing uses, or a translation drops a `{value}`.
//!
//! Glossary (agreed 2026-11-15): widget → widget, dashboard → Kontrollpanel,
//! Theme Studio → Temastudio, taskbar → oppgavelinjen, notification area →
//! systemstatusfeltet, usage providers / AI usage → KI-forbruk (AI → KI),
//! Pin to taskbar / desktop → Fest til oppgavelinjen / skrivebordet, Float on
//! top → Flyt øverst, magnetic placement → Magnetisk plassering, Now Playing
//! → Spilles nå, Next Departure → Neste avgang, Mail → E-post, Session /
//! Weekly → Økt / Uke.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// The language setting in `widgets.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    /// Norwegian when Windows' display language is Norwegian, else English.
    #[default]
    System,
    English,
    Norwegian,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::System, Self::English, Self::Norwegian];

    /// The name shown in the language picker. Each language is named in
    /// itself, so it can be found whatever language is showing.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => tr("Same as Windows"),
            Self::English => "English",
            Self::Norwegian => "Norsk bokmål",
        }
    }
}

#[cfg(not(test))]
static NORWEGIAN: AtomicBool = AtomicBool::new(false);

// Tests run in parallel threads: each keeps its own language, so a test
// that switches to Norwegian cannot change another test's English text.
#[cfg(test)]
thread_local! {
    static NORWEGIAN: AtomicBool = const { AtomicBool::new(false) };
}

/// Switches the interface to `choice`. Cheap: call it once per frame.
pub fn apply(choice: Language) {
    let nb = match choice {
        Language::System => windows_is_norwegian(),
        Language::English => false,
        Language::Norwegian => true,
    };
    #[cfg(not(test))]
    NORWEGIAN.store(nb, Ordering::Relaxed);
    #[cfg(test)]
    NORWEGIAN.with(|n| n.store(nb, Ordering::Relaxed));
}

pub fn norwegian() -> bool {
    #[cfg(not(test))]
    return NORWEGIAN.load(Ordering::Relaxed);
    #[cfg(test)]
    return NORWEGIAN.with(|n| n.load(Ordering::Relaxed));
}

/// Windows' display language is Norwegian (bokmål, nynorsk or plain "no").
fn windows_is_norwegian() -> bool {
    static NB: OnceLock<bool> = OnceLock::new();
    *NB.get_or_init(|| {
        // LANG_NORWEGIAN is the primary language of nb-NO and nn-NO.
        const LANG_NORWEGIAN: u16 = 0x14;
        let lang = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        lang & 0x3ff == LANG_NORWEGIAN
    })
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| crate::i18n_nb::NB.iter().copied().collect())
}

/// `en` in the current language.
pub fn tr(en: &'static str) -> &'static str {
    if norwegian() {
        table().get(en).copied().unwrap_or(en)
    } else {
        en
    }
}

/// `en` in the current language, with each `{name}` replaced by its value.
pub fn trf(en: &'static str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = tr(en).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every string literal passed to `tr(` or `trf(` in `src`.
    fn used() -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    // As rustc does: a line continuation then works the same in
                    // files with Windows line endings.
                    let text = std::fs::read_to_string(&path)
                        .unwrap()
                        .replace("\r\n", "\n");
                    found.extend(literals(&text));
                }
            }
        }
        found
    }

    fn literals(text: &str) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(at) = text[i..].find("tr") {
            let start = i + at;
            i = start + 2;
            let before = start.checked_sub(1).map(|b| bytes[b]);
            if before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
                continue;
            }
            let mut j = i;
            if bytes.get(j) == Some(&b'f') {
                j += 1;
            }
            if bytes.get(j) != Some(&b'(') {
                continue;
            }
            j += 1;
            while bytes.get(j).is_some_and(|b| b.is_ascii_whitespace()) {
                j += 1;
            }
            if bytes.get(j) != Some(&b'"') {
                continue;
            }
            j += 1;
            let mut s = String::new();
            let mut chars = text[j..].chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => match chars.next() {
                        Some('n') => s.push('\n'),
                        Some('\n') => {
                            // Line continuation: skip the next line's indent.
                            let rest: String = chars.clone().collect();
                            let skip = rest.len() - rest.trim_start().len();
                            for _ in 0..rest[..skip].chars().count() {
                                chars.next();
                            }
                        }
                        Some(other) => s.push(other),
                        None => break,
                    },
                    c => s.push(c),
                }
            }
            out.push(s);
        }
        out
    }

    fn placeholders(s: &str) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        let mut rest = s;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            set.insert(rest[open + 1..open + close].to_string());
            rest = &rest[open + close + 1..];
        }
        set
    }

    #[test]
    fn every_text_has_a_norwegian_translation_and_none_is_unused() {
        let used = used();
        assert!(used.len() > 100, "the scanner found only {}", used.len());
        let table = table();
        let missing: Vec<_> = used
            .iter()
            .filter(|s| !table.contains_key(s.as_str()))
            .collect();
        assert!(missing.is_empty(), "no Norwegian for: {missing:#?}");
        let unused: Vec<_> = table.keys().filter(|k| !used.contains(**k)).collect();
        assert!(unused.is_empty(), "unused translations: {unused:#?}");
        assert_eq!(
            table.len(),
            crate::i18n_nb::NB.len(),
            "a text is translated twice"
        );
    }

    #[test]
    fn translations_keep_every_value() {
        for (en, nb) in crate::i18n_nb::NB {
            assert_eq!(placeholders(en), placeholders(nb), "{en} → {nb}");
        }
    }

    #[test]
    fn the_scanner_reads_escapes_and_continued_lines() {
        let src =
            "x(tr(\"a \\\"b\\\"\")); trf(\n  \"c {d} \\\n   e\", &[]); str(\"no\"); attr(\"no\")";
        assert_eq!(literals(src), vec!["a \"b\"", "c {d} e"]);
    }

    #[test]
    fn values_are_filled_in_after_translating() {
        apply(Language::Norwegian);
        assert_eq!(trf("{count} unread", &[("count", &5)]), "5 uleste");
        apply(Language::English);
        assert_eq!(trf("{count} unread", &[("count", &5)]), "5 unread");
    }
}
