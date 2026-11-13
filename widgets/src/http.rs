//! ureq agents using the Windows certificate store via native-tls.

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

fn build(skip_verification: bool, webdav: bool) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .disable_verification(skip_verification)
        .build();
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(if webdav { 90 } else { 30 })))
        .tls_config(tls)
        // PROPFIND / REPORT are not in ureq's built-in method list.
        .allow_non_standard_methods(webdav)
        // WebDAV callers read the status themselves to report it.
        .http_status_as_error(!webdav)
        .build()
        .into()
}

/// Agent for public HTTPS APIs (Yr). Certificates are always verified.
pub fn agent() -> ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| build(false, false)).clone()
}

/// Identifies the app to MET Norway and Yr, as their terms of service ask.
pub const USER_AGENT: &str = concat!(
    "BetterTaskbar/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/mrk1987/bettertaskbar)"
);

/// The only places `get_isolated` may fetch from.
const ISOLATED_PREFIXES: [&str; 2] = ["https://api.met.no/", "https://www.yr.no/"];

fn may_isolate(url: &str) -> bool {
    ISOLATED_PREFIXES.iter().any(|p| url.starts_with(p))
}

/// GET `url` in a short-lived copy of this program and return the body.
///
/// One HTTPS request to api.met.no left about 20 MiB more in the calling
/// process for good (measured 2026-11-13: 53 MiB private working set before,
/// 74 MiB after, with or without certificate checks). A pill lives for days
/// and fetches a few kilobytes now and then, so the request runs in a child
/// that exits and gives the memory back.
pub fn get_isolated(url: &str) -> Option<Vec<u8>> {
    if !may_isolate(url) {
        return None;
    }
    let output = std::process::Command::new(std::env::current_exe().ok()?)
        .arg(format!("--http-get={url}"))
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// The child side of `get_isolated`: write the body to stdout. Returns the
/// process exit code.
pub fn serve_get(url: &str) -> i32 {
    use std::io::Write;
    if !may_isolate(url) {
        return 2;
    }
    let body = agent()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .ok()
        .and_then(|mut response| response.body_mut().read_to_vec().ok());
    match body {
        Some(body) if std::io::stdout().write_all(&body).is_ok() => 0,
        _ => 1,
    }
}

/// Agent for a CalDAV server. Certificate checks are skipped only for a host on
/// this machine or a private network, where bridges use self-signed certs.
pub fn webdav_agent(host: &str) -> ureq::Agent {
    build(is_private_host(host), true)
}

/// True for `localhost` and for IP literals that are loopback, RFC 1918,
/// link-local, CGNAT (100.64/10, which Tailscale uses) or IPv6 unique-local.
/// Any other name or address is treated as public and gets full TLS checks.
pub fn is_private_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            let o = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || (o[0] == 100 && (o[1] & 0xC0) == 64)
        }
        Ok(IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            ip.is_loopback() || (first & 0xFE00) == 0xFC00 || (first & 0xFFC0) == 0xFE80
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_private_host, may_isolate};

    #[test]
    fn only_weather_hosts_are_fetched_in_a_child() {
        assert!(may_isolate(
            "https://api.met.no/weatherapi/locationforecast/2.0/compact?lat=1&lon=2"
        ));
        assert!(may_isolate(
            "https://www.yr.no/assets/images/weather-symbols/dark-mode/default/svg/01d.svg"
        ));
        assert!(!may_isolate("https://api.met.no.evil.example/x"));
        assert!(!may_isolate("http://api.met.no/x"));
        assert!(!may_isolate("file:///C:/Windows/win.ini"));
    }

    #[test]
    fn private_hosts() {
        for h in [
            "127.0.0.1",
            "localhost",
            "192.168.1.10",
            "10.0.0.1",
            "172.16.5.5",
            "100.64.0.1",
            "::1",
            "[fd7a::1]",
        ] {
            assert!(is_private_host(h), "{h}");
        }
        for h in [
            "8.8.8.8",
            "100.128.0.1",
            "172.32.0.1",
            "example.com",
            "2606:4700::1111",
            "calendar.proton.me",
        ] {
            assert!(!is_private_host(h), "{h}");
        }
    }
}
