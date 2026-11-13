//! Minimal IMAP client for the mail pill: connect (STARTTLS, implicit TLS or
//! plain), LOGIN, read-only EXAMINE INBOX, UID SEARCH UNSEEN, then fetch the
//! From/Subject headers of the newest unread messages.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use base64::Engine;

use crate::config::MailCfg;

/// Headers are fetched for at most this many of the newest unread messages.
const MAX_SUBJECTS: usize = 30;
const TIMEOUT: Duration = Duration::from_secs(20);
/// Larger literals, or more untagged lines per command, end the session.
const MAX_LITERAL: usize = 1024 * 1024;
const MAX_LINES: usize = 10_000;

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

type Stream = BufReader<Box<dyn ReadWrite>>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unread {
    /// Total unread in INBOX; may exceed `subjects.len()`.
    pub count: usize,
    /// Display lines, newest first: the subject, or the sender if it has none.
    pub subjects: Vec<String>,
}

pub fn fetch(cfg: &MailCfg) -> Result<Unread, String> {
    let mut s = connect(cfg)?;
    let mut tag = 0u32;
    let mut next = || {
        tag += 1;
        format!("a{tag:03}")
    };

    let login = format!(
        "LOGIN \"{}\" \"{}\"",
        quote(&cfg.username),
        quote(&cfg.password)
    );
    command(&mut s, &next(), &login).map_err(|e| format!("login failed: {e}"))?;
    // EXAMINE is SELECT without write access: nothing can be marked as read.
    command(&mut s, &next(), "EXAMINE INBOX").map_err(|e| format!("opening INBOX failed: {e}"))?;

    let found =
        command(&mut s, &next(), "UID SEARCH UNSEEN").map_err(|e| format!("search failed: {e}"))?;
    let mut uids: Vec<u32> = found
        .lines
        .iter()
        .filter_map(|l| strip_prefix_ci(l, "* SEARCH"))
        .flat_map(|rest| {
            rest.split_whitespace()
                .filter_map(|n| n.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect();
    uids.sort_unstable();
    let mut unread = Unread {
        count: uids.len(),
        subjects: vec![],
    };

    if !uids.is_empty() {
        let newest: Vec<String> = uids
            .iter()
            .rev()
            .take(MAX_SUBJECTS)
            .rev()
            .map(u32::to_string)
            .collect();
        let fetched = command(
            &mut s,
            &next(),
            &format!(
                "UID FETCH {} (BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])",
                newest.join(",")
            ),
        )
        .map_err(|e| format!("fetching headers failed: {e}"))?;
        // Servers answer in mailbox order (oldest first); show newest first.
        unread.subjects = fetched
            .literals
            .iter()
            .rev()
            .filter_map(|h| display_line(h))
            .collect();
    }
    let _ = command(&mut s, &next(), "LOGOUT");
    Ok(unread)
}

fn display_line(headers: &str) -> Option<String> {
    let subject = header(headers, "Subject").unwrap_or_default();
    if !subject.is_empty() {
        return Some(subject);
    }
    let from = header(headers, "From").unwrap_or_default();
    // "Name <addr>" reads better as just the name.
    let name = from
        .split('<')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .trim();
    let shown = if name.is_empty() {
        from.trim_matches(|c| c == '<' || c == '>')
    } else {
        name
    };
    (!shown.is_empty()).then(|| shown.to_owned())
}

/// Open the transport, read the greeting, and upgrade with STARTTLS if asked.
fn connect(cfg: &MailCfg) -> Result<Stream, String> {
    let target = format!("{}:{}", cfg.host, cfg.port);
    let addr = (cfg.host.as_str(), cfg.port)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {target}: {e}"))?
        .next()
        .ok_or_else(|| format!("cannot resolve {target}"))?;
    let tcp = TcpStream::connect_timeout(&addr, TIMEOUT)
        .map_err(|e| format!("cannot connect to {target}: {e}"))?;
    let _ = tcp.set_read_timeout(Some(TIMEOUT));
    let _ = tcp.set_write_timeout(Some(TIMEOUT));

    let greet = |s: &mut dyn BufRead| -> Result<(), String> {
        let line = read_line(s)?;
        if line.starts_with("* OK") || line.starts_with("* PREAUTH") {
            Ok(())
        } else {
            Err(format!("unexpected greeting from {target}: {line}"))
        }
    };

    match cfg.security.to_ascii_lowercase().as_str() {
        "tls" | "ssl" => {
            let mut s: Stream = BufReader::new(Box::new(tls(cfg, tcp)?));
            greet(&mut s)?;
            Ok(s)
        }
        "starttls" => {
            // Handshake on the plain stream, then wrap the same socket.
            let mut plain = BufReader::new(tcp);
            greet(&mut plain)?;
            send(plain.get_mut(), "a000 STARTTLS")?;
            loop {
                let line = read_line(&mut plain)?;
                if let Some(rest) = line.strip_prefix("a000 ") {
                    if !starts_with_ci(rest, "OK") {
                        return Err(format!("server refused STARTTLS: {rest}"));
                    }
                    break;
                }
            }
            // Nothing may be buffered past the OK, or it would bypass TLS.
            if !plain.buffer().is_empty() {
                return Err("unexpected data after STARTTLS".to_owned());
            }
            Ok(BufReader::new(Box::new(tls(cfg, plain.into_inner())?)))
        }
        "plain" | "none" => {
            let mut s: Stream = BufReader::new(Box::new(tcp));
            greet(&mut s)?;
            Ok(s)
        }
        other => Err(format!(
            "mail.security is \"{other}\"; use starttls, tls or plain"
        )),
    }
}

/// TLS to the mail server. Certificate checks are skipped only for a host on
/// this machine or a private network (Proton Bridge uses a self-signed cert).
fn tls(cfg: &MailCfg, tcp: TcpStream) -> Result<native_tls::TlsStream<TcpStream>, String> {
    let local = crate::http::is_private_host(&cfg.host);
    native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(local)
        .danger_accept_invalid_hostnames(local)
        .build()
        .map_err(|e| format!("TLS setup failed: {e}"))?
        .connect(&cfg.host, tcp)
        .map_err(|e| format!("TLS handshake with {} failed: {e}", cfg.host))
}

/// Escape for an IMAP quoted string. CR and LF cannot appear in one, and would
/// let a value smuggle in a second command, so they are dropped.
fn quote(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\r' && *c != '\n')
        .flat_map(|c| match c {
            '\\' | '"' => vec!['\\', c],
            c => vec![c],
        })
        .collect()
}

fn send(w: &mut impl Write, line: &str) -> Result<(), String> {
    w.write_all(format!("{line}\r\n").as_bytes())
        .and_then(|_| w.flush())
        .map_err(|e| format!("send failed: {e}"))
}

fn read_line(s: &mut (impl BufRead + ?Sized)) -> Result<String, String> {
    let mut raw = Vec::new();
    match s.read_until(b'\n', &mut raw) {
        Ok(0) => Err("the server closed the connection".to_owned()),
        Ok(_) => Ok(String::from_utf8_lossy(&raw).trim_end().to_owned()),
        Err(e) => Err(format!("read failed: {e}")),
    }
}

#[derive(Debug, Default)]
struct Response {
    /// Untagged response lines.
    lines: Vec<String>,
    /// Bodies of `{n}` literals, in order of arrival.
    literals: Vec<String>,
}

/// Send a tagged command and collect the response up to its tagged completion.
/// Anything but a tagged OK is an error carrying the server's own text.
fn command(s: &mut Stream, tag: &str, text: &str) -> Result<Response, String> {
    send(s.get_mut(), &format!("{tag} {text}"))?;
    let mut response = Response::default();
    loop {
        let line = read_line(s)?;
        if let Some(rest) = line.strip_prefix(tag).and_then(|r| r.strip_prefix(' ')) {
            return if starts_with_ci(rest, "OK") {
                Ok(response)
            } else {
                Err(rest.to_owned())
            };
        }
        if let Some(n) = literal_len(&line) {
            // The number comes from the server and sizes an allocation. A
            // literal that is not read would be taken for response lines, and
            // one of them could pass for the tagged completion.
            if n > MAX_LITERAL {
                return Err(format!("the server sent a {n}-byte literal"));
            }
            let mut buf = vec![0u8; n];
            s.read_exact(&mut buf)
                .map_err(|e| format!("read failed: {e}"))?;
            response
                .literals
                .push(String::from_utf8_lossy(&buf).into_owned());
        }
        if response.lines.len() >= MAX_LINES {
            return Err("the server sent too many response lines".to_owned());
        }
        response.lines.push(line);
    }
}

fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    starts_with_ci(s, prefix).then(|| &s[prefix.len()..])
}

/// Size of the literal a line announces with a trailing `{n}`.
fn literal_len(line: &str) -> Option<usize> {
    let inner = line.strip_suffix('}')?;
    let n = &inner[inner.rfind('{')? + 1..];
    n.trim_end_matches('+').parse().ok()
}

/// A header's value with continuation lines unfolded and encoded words decoded.
fn header(block: &str, name: &str) -> Option<String> {
    let mut value: Option<String> = None;
    for line in block.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(v) = value.as_mut() {
                v.push(' ');
                v.push_str(line.trim());
            }
        } else if value.is_some() {
            break;
        } else if let Some((key, rest)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                value = Some(rest.trim().to_owned());
            }
        }
    }
    value.map(|v| decode_words(&v))
}

/// Decode RFC 2047 encoded words: `=?charset?Q|B?text?=`.
fn decode_words(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    let mut after_word = false;
    while let Some(start) = rest.find("=?") {
        let (before, candidate) = rest.split_at(start);
        match decode_word(candidate) {
            Some((text, len)) => {
                // White space between two encoded words is not part of the text.
                if !(after_word && before.trim().is_empty()) {
                    out.push_str(before);
                }
                out.push_str(&text);
                rest = &candidate[len..];
                after_word = true;
            }
            None => {
                out.push_str(before);
                out.push_str("=?");
                rest = &candidate[2..];
                after_word = false;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Decode one encoded word at the start of `s`; returns the text and how many
/// bytes of `s` it took. The parts are split on `?` in order, so a Q-encoded
/// text that begins with `=` (as in `?Q?=C3=B8`) is not mistaken for the end.
fn decode_word(s: &str) -> Option<(String, usize)> {
    let body = s.strip_prefix("=?")?;
    let (charset, body) = body.split_once('?')?;
    let (encoding, body) = body.split_once('?')?;
    let end = body.find("?=")?;
    let text = &body[..end];
    if charset.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let bytes = match encoding {
        "Q" | "q" => q_decode(text),
        "B" | "b" => base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(text.trim_end_matches('='))
            .ok()?,
        _ => return None,
    };
    let used = 2 + charset.len() + 1 + encoding.len() + 1 + end + 2;
    Some((to_text(charset, &bytes), used))
}

fn to_text(charset: &str, bytes: &[u8]) -> String {
    // "charset*language" (RFC 2231) is allowed; only the charset matters.
    let charset = charset
        .split('*')
        .next()
        .unwrap_or(charset)
        .to_ascii_lowercase();
    match charset.as_str() {
        // Windows-1252 differs from Latin-1 only in 0x80-0x9F (curly quotes,
        // dashes, the euro sign), which the table below covers.
        "iso-8859-1" | "latin1" | "iso-8859-15" | "windows-1252" | "cp1252" => {
            bytes.iter().map(|&b| cp1252(b)).collect()
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

fn q_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        // "=XX" needs two more bytes after the '='.
        let hex = (b[i] == b'=' && i + 2 < b.len())
            .then(|| {
                std::str::from_utf8(&b[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
            })
            .flatten();
        match (b[i], hex) {
            (_, Some(v)) => {
                out.push(v);
                i += 3;
            }
            (b'_', _) => {
                out.push(b' ');
                i += 1;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_word_starting_with_an_encoded_byte() {
        assert_eq!(decode_words("=?UTF-8?Q?=C3=98l_og_mat?="), "Øl og mat");
        assert_eq!(decode_words("=?utf-8?q?Hei=2C_verden?="), "Hei, verden");
    }

    #[test]
    fn q_escape_at_the_very_end() {
        assert_eq!(decode_words("=?UTF-8?Q?bl=C3=A5?="), "blå");
    }

    #[test]
    fn b_words_and_padding() {
        assert_eq!(decode_words("=?UTF-8?B?SMOlbGxv?="), "Hållo");
        assert_eq!(decode_words("=?UTF-8?B?w6U=?="), "å");
        assert_eq!(decode_words("=?UTF-8?B?w6XDpQ==?="), "åå");
    }

    #[test]
    fn latin1_and_cp1252() {
        assert_eq!(decode_words("=?ISO-8859-1?Q?Bl=E5b=E6r?="), "Blåbær");
        assert_eq!(decode_words("=?windows-1252?Q?5_=80_=96_ok?="), "5 € – ok");
    }

    #[test]
    fn mixed_and_adjacent_words() {
        assert_eq!(
            decode_words("Re: =?UTF-8?Q?M=C3=B8te?= i morgen"),
            "Re: Møte i morgen"
        );
        // The space between two encoded words is dropped, other spaces stay.
        assert_eq!(
            decode_words("=?UTF-8?Q?M=C3=B8?= =?UTF-8?Q?te?= kl 9"),
            "Møte kl 9"
        );
    }

    #[test]
    fn things_that_only_look_encoded_are_left_alone() {
        assert_eq!(decode_words("a =? b ?= c"), "a =? b ?= c");
        assert_eq!(decode_words("=?UTF-8?X?abc?="), "=?UTF-8?X?abc?=");
        assert_eq!(decode_words("plain subject"), "plain subject");
        assert_eq!(decode_words("=?UTF-8?B?!!!?="), "=?UTF-8?B?!!!?=");
    }

    #[test]
    fn headers_any_case_and_folded() {
        let block =
            "FROM: \"Kari Nordmann\" <kari@example.com>\r\nsubject: En lang\r\n tittel\r\n\r\n";
        assert_eq!(header(block, "Subject").as_deref(), Some("En lang tittel"));
        assert_eq!(
            header(block, "From").as_deref(),
            Some("\"Kari Nordmann\" <kari@example.com>")
        );
        assert_eq!(display_line(block).as_deref(), Some("En lang tittel"));
    }

    #[test]
    fn sender_is_shown_when_there_is_no_subject() {
        assert_eq!(
            display_line("From: \"Kari Nordmann\" <kari@example.com>\r\nSubject:\r\n").as_deref(),
            Some("Kari Nordmann")
        );
        assert_eq!(
            display_line("From: <kari@example.com>\r\n").as_deref(),
            Some("kari@example.com")
        );
        assert_eq!(display_line("\r\n"), None);
    }

    #[test]
    fn quoting() {
        assert_eq!(quote(r#"pa"ss\word"#), r#"pa\"ss\\word"#);
        assert_eq!(quote("x\r\na001 LOGOUT"), "xa001 LOGOUT");
    }

    #[test]
    fn literals() {
        assert_eq!(
            literal_len("* 1 FETCH (UID 7 BODY[HEADER.FIELDS (FROM SUBJECT)] {123}"),
            Some(123)
        );
        assert_eq!(literal_len("* OK done"), None);
        assert!(literal_len("* 1 FETCH {99999999999}").is_some_and(|n| n > MAX_LITERAL));
    }

    #[test]
    fn an_oversized_literal_ends_the_session_instead_of_being_read_as_lines() {
        // Unread, the literal's own text would pass for the tagged completion.
        let mut s = stream("* 1 FETCH (BODY[] {2000000}\r\na001 OK spoofed\r\n");
        let error = command(&mut s, "a001", "UID FETCH 1 (BODY.PEEK[])").unwrap_err();
        assert!(error.contains("2000000-byte literal"), "{error}");
    }

    fn stream(server: &str) -> Stream {
        struct Fake(std::io::Cursor<Vec<u8>>);
        impl Read for Fake {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(buf)
            }
        }
        impl Write for Fake {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        BufReader::new(Box::new(Fake(std::io::Cursor::new(
            server.as_bytes().to_vec(),
        ))))
    }

    #[test]
    fn command_collects_literals_and_stops_at_the_tag() {
        let mut s = stream("* 1 FETCH (UID 7 BODY[HEADER.FIELDS (FROM SUBJECT)] {15}\r\nSubject: Hi\r\n\r\n)\r\na001 OK done\r\n");
        let r = command(&mut s, "a001", "UID FETCH 7 (...)").unwrap();
        assert_eq!(r.literals, vec!["Subject: Hi\r\n\r\n"]);
    }

    #[test]
    fn command_reports_no_and_a_closed_connection() {
        let mut s = stream("a001 NO [AUTHENTICATIONFAILED] no such user\r\n");
        assert_eq!(
            command(&mut s, "a001", "LOGIN x y").unwrap_err(),
            "NO [AUTHENTICATIONFAILED] no such user"
        );
        // An untagged line containing "OK" is not a completion, and EOF ends the wait.
        let mut s = stream("* OK still here\r\n");
        assert_eq!(
            command(&mut s, "a001", "NOOP").unwrap_err(),
            "the server closed the connection"
        );
    }
}
