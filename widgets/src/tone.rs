//! Read-only taskbar tone detection. Unknown settings fall back to a dark bar.

use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, REG_ROUTINE_FLAGS, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD,
};

pub const LIGHT_THRESHOLD: f32 = 0.4;
const REFRESH: Duration = Duration::from_secs(3);
static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

/// WCAG relative luminance, with sRGB channels converted to linear light.
pub fn relative_luminance(rgb: [u8; 3]) -> f32 {
    let [r, g, b] = rgb.map(|v| {
        let s = f32::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG contrast ratio for opaque text and a solid reference background.
#[cfg(test)]
pub fn contrast(a: eframe::egui::Color32, b: [u8; 3]) -> f32 {
    let a = relative_luminance([a.r(), a.g(), a.b()]);
    let b = relative_luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

pub fn bar_is_light(uses_light_theme: bool, accent_on_bar: bool, accent: Option<[u8; 3]>) -> bool {
    if accent_on_bar {
        if let Some(rgb) = accent {
            return relative_luminance(rgb) > LIGHT_THRESHOLD;
        }
    }
    uses_light_theme
}

/// Read exactly one value of the expected type and size; malformed data is ignored.
fn read_value<const N: usize>(
    key: PCWSTR,
    name: PCWSTR,
    flags: REG_ROUTINE_FLAGS,
) -> Option<[u8; N]> {
    let mut data = [0; N];
    let mut size = N as u32;
    // SAFETY: both registry strings are static, nul-terminated wide strings;
    // the output buffer has the size supplied to RegGetValueW and lives for the call.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            flags,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (result.is_ok() && size == N as u32).then_some(data)
}

fn read_taskbar_is_light() -> bool {
    let personalize = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let dword = |name| {
        read_value::<4>(personalize, name, RRF_RT_REG_DWORD)
            .map(u32::from_le_bytes)
            .unwrap_or(0)
            != 0
    };
    let accent = read_value::<32>(
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"),
        w!("AccentPalette"),
        RRF_RT_REG_BINARY,
    )
    .map(|p| [p[20], p[21], p[22]]);
    bar_is_light(
        dword(w!("SystemUsesLightTheme")),
        dword(w!("ColorPrevalence")),
        accent,
    )
}

fn cached_tone(
    cache: &mut Option<(Instant, bool)>,
    now: Instant,
    read: impl FnOnce() -> bool,
) -> bool {
    if let Some((at, light)) = *cache {
        if now.duration_since(at) <= REFRESH {
            return light;
        }
    }
    let light = read();
    *cache = Some((now, light));
    light
}

fn parse_force_light_bar(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Screenshot/test aid; only the exact value `1` overrides Windows' tone.
pub fn force_light_bar() -> bool {
    parse_force_light_bar(std::env::var("BTW_FORCE_LIGHT_BAR").ok().as_deref())
}

pub fn taskbar_is_light() -> bool {
    if force_light_bar() {
        return true;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cached_tone(&mut cache, Instant::now(), read_taskbar_is_light)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_light_bar_requires_exactly_one() {
        assert!(parse_force_light_bar(Some("1")));
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("true"),
            Some(" 1"),
            Some("1 "),
            Some("01"),
        ] {
            assert!(!parse_force_light_bar(value), "{value:?}");
        }
    }

    #[test]
    fn black_and_white_contrast_is_twenty_one() {
        assert!((contrast(eframe::egui::Color32::BLACK, [255; 3]) - 21.0).abs() < 0.001);
        assert!((contrast(eframe::egui::Color32::WHITE, [0; 3]) - 21.0).abs() < 0.001);
    }

    #[test]
    fn cache_refreshes_after_three_seconds() {
        let now = Instant::now();
        let mut cache = None;
        assert!(!cached_tone(&mut cache, now, || false));
        assert!(!cached_tone(&mut cache, now + REFRESH, || panic!(
            "cache is still fresh"
        )));
        assert!(cached_tone(
            &mut cache,
            now + REFRESH + Duration::from_millis(1),
            || true
        ));
        assert!(cached_tone(
            &mut cache,
            now + REFRESH + Duration::from_secs(1),
            || panic!("refreshed cache is still fresh")
        ));
    }

    #[test]
    fn luminance_endpoints() {
        assert!(relative_luminance([0, 0, 0]) < 0.001);
        assert!(relative_luminance([255, 255, 255]) > 0.999);
    }

    #[test]
    fn plain_light_and_dark_bars() {
        assert!(bar_is_light(true, false, None));
        assert!(!bar_is_light(false, false, None));
    }

    #[test]
    fn accent_on_bar_decides_by_its_brightness() {
        assert!(bar_is_light(false, true, Some([0xFF, 0xD7, 0x00])));
        assert!(!bar_is_light(true, true, Some([0x00, 0x33, 0x66])));
        assert!(bar_is_light(true, true, None));
    }
}
