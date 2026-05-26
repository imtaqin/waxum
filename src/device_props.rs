//! Device props resolver — controls how each session appears in WhatsApp's
//! "Linked Devices" list at pair time, and what the WA server sees as the
//! companion's client fingerprint.
//!
//! Defaults: `os = "Windows"`, `platform = Chrome`, `version = 2.3000.1023902713`
//! (a current-ish WA Web build). The version is critical — if it stays at the
//! upstream lib default of `0.1.0`, WA server can flag the device as a fake
//! client and silently drop outbound messages.
//!
//! Override globally via env:
//!   `WA_DEVICE_OS`        — e.g. "Windows", "Mac OS X", "Ubuntu"
//!   `WA_DEVICE_PLATFORM`  — one of: chrome, firefox, edge, safari, opera,
//!                          ie, desktop, ipad, android_phone, android_tablet
//!   `WA_DEVICE_VERSION`   — dotted version, e.g. "2.3000.1023902713"
//!
//! These only take effect at the first pairing — once the device is registered,
//! whatsapp-rust persists the props in its SQLite store and re-uses them on
//! subsequent connects.

use waproto::whatsapp as wa;

#[derive(Debug, Clone)]
pub struct ResolvedDeviceProps {
    pub os: String,
    pub platform: wa::device_props::PlatformType,
    pub version: wa::device_props::AppVersion,
}

pub fn resolve_from_env() -> ResolvedDeviceProps {
    let os = std::env::var("WA_DEVICE_OS").unwrap_or_else(|_| "Windows".to_string());
    let platform = std::env::var("WA_DEVICE_PLATFORM")
        .ok()
        .as_deref()
        .map(parse_platform)
        .unwrap_or(wa::device_props::PlatformType::Chrome);
    let version = std::env::var("WA_DEVICE_VERSION")
        .ok()
        .as_deref()
        .and_then(parse_version)
        .unwrap_or_else(default_app_version);
    ResolvedDeviceProps { os, platform, version }
}

fn parse_platform(s: &str) -> wa::device_props::PlatformType {
    use wa::device_props::PlatformType as P;
    match s.trim().to_ascii_lowercase().as_str() {
        "chrome" => P::Chrome,
        "firefox" => P::Firefox,
        "edge" => P::Edge,
        "safari" => P::Safari,
        "opera" => P::Opera,
        "ie" => P::Ie,
        "desktop" => P::Desktop,
        "ipad" => P::Ipad,
        "android_phone" | "android" => P::AndroidPhone,
        "android_tablet" => P::AndroidTablet,
        "ios_phone" | "iphone" => P::IosPhone,
        _ => P::Chrome,
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

/// Default version aligned with a recent WA Web build. Bump occasionally
/// to stay close to the live client — too far behind and the server may
/// throttle or flag the device.
pub fn default_app_version() -> wa::device_props::AppVersion {
    wa::device_props::AppVersion {
        primary: Some(2),
        secondary: Some(3000),
        tertiary: Some(1023902713),
        quaternary: None,
        quinary: None,
    }
}
