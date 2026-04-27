//! Device props resolver — controls how each session appears in WhatsApp's
//! "Linked Devices" list at pair time.
//!
//! Defaults: `os = "Windows"`, `platform = Chrome`. Override globally via env:
//!   `WA_DEVICE_OS`        — e.g. "Windows", "Mac OS X", "Ubuntu"
//!   `WA_DEVICE_PLATFORM`  — one of: chrome, firefox, edge, safari, opera,
//!                          ie, desktop, ipad, android_phone, android_tablet
//!
//! These only take effect at the first pairing — once the device is registered,
//! whatsapp-rust persists the props in its SQLite store and re-uses them on
//! subsequent connects.

use waproto::whatsapp as wa;

#[derive(Debug, Clone)]
pub struct ResolvedDeviceProps {
    pub os: String,
    pub platform: wa::device_props::PlatformType,
}

pub fn resolve_from_env() -> ResolvedDeviceProps {
    let os = std::env::var("WA_DEVICE_OS").unwrap_or_else(|_| "Windows".to_string());
    let platform = std::env::var("WA_DEVICE_PLATFORM")
        .ok()
        .as_deref()
        .map(parse_platform)
        .unwrap_or(wa::device_props::PlatformType::Chrome);
    ResolvedDeviceProps { os, platform }
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
