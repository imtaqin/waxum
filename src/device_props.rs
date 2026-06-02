//! Device props resolver — controls how each session appears in WhatsApp's
//! "Linked Devices" list at pair time.
//!
//! Defaults: `os = "Windows"`, `platform = Desktop` (shows as "WhatsApp
//! Desktop" in Linked Devices, not a browser). AppVersion is left at
//! whatever the upstream lib provides (currently 0.1.0); empirical testing
//! showed that forcing a "realistic" 2.3000.x build number caused WA's
//! server to silently drop outbound messages on freshly-paired sessions,
//! while the upstream default delivers fine. Opt in via env only.
//!
//! Override globally via env:
//!   `WA_DEVICE_OS`        — e.g. "Windows", "Mac OS X", "Ubuntu"
//!   `WA_DEVICE_PLATFORM`  — one of: desktop, uwp, chrome, firefox, edge,
//!                          safari, opera, ie, ipad, android_phone,
//!                          android_tablet, ios_phone
//!   `WA_DEVICE_VERSION`   — dotted version, e.g. "2.3000.1023902713".
//!                          Unset = lib default (recommended).
//!
//! These only take effect at the first pairing — once the device is registered,
//! whatsapp-rust persists the props in its SQLite store and re-uses them on
//! subsequent connects.

use waproto::whatsapp as wa;

#[derive(Debug, Clone)]
pub struct ResolvedDeviceProps {
    pub os: String,
    pub platform: wa::device_props::PlatformType,
    /// `None` means "let whatsapp-rust pick the default version" — preferred
    /// in production. Set only when you have evidence the server expects a
    /// specific build (set via `WA_DEVICE_VERSION`).
    pub version: Option<wa::device_props::AppVersion>,
}

pub fn resolve_from_env() -> ResolvedDeviceProps {
    let os = std::env::var("WA_DEVICE_OS").unwrap_or_else(|_| "Windows".to_string());
    let platform = std::env::var("WA_DEVICE_PLATFORM")
        .ok()
        .as_deref()
        .map(parse_platform)
        .unwrap_or(wa::device_props::PlatformType::Desktop);
    let version = std::env::var("WA_DEVICE_VERSION")
        .ok()
        .as_deref()
        .and_then(parse_version);
    ResolvedDeviceProps {
        os,
        platform,
        version,
    }
}

/// Resolve device props with an optional per-request override layered on
/// top of env defaults. Each field in `override_` only takes effect when
/// `Some`; missing fields fall back to whatever `resolve_from_env` produced.
pub fn resolve_with_override(
    os: Option<&str>,
    platform: Option<&str>,
    version: Option<&str>,
) -> ResolvedDeviceProps {
    let mut base = resolve_from_env();
    if let Some(o) = os.and_then(non_empty) {
        base.os = o.to_string();
    }
    if let Some(p) = platform.and_then(non_empty) {
        base.platform = parse_platform(p);
    }
    if let Some(v) = version.and_then(non_empty) {
        base.version = parse_version(v);
    }
    base
}

fn non_empty(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn parse_platform(s: &str) -> wa::device_props::PlatformType {
    use wa::device_props::PlatformType as P;
    match s.trim().to_ascii_lowercase().as_str() {
        "desktop" => P::Desktop,
        "uwp" | "windows_store" => P::Uwp,
        "chrome" => P::Chrome,
        "firefox" => P::Firefox,
        "edge" => P::Edge,
        "safari" => P::Safari,
        "opera" => P::Opera,
        "ie" => P::Ie,
        "ipad" => P::Ipad,
        "android_phone" | "android" => P::AndroidPhone,
        "android_tablet" => P::AndroidTablet,
        "ios_phone" | "iphone" => P::IosPhone,
        _ => P::Desktop,
    }
}

/// Parse a dotted version like "2.3000.1023902713" into a proto AppVersion.
fn parse_version(s: &str) -> Option<wa::device_props::AppVersion> {
    let parts: Vec<u32> = s.trim().split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.is_empty() {
        return None;
    }
    Some(wa::device_props::AppVersion {
        primary: parts.first().copied(),
        secondary: parts.get(1).copied(),
        tertiary: parts.get(2).copied(),
        quaternary: parts.get(3).copied(),
        quinary: parts.get(4).copied(),
    })
}
