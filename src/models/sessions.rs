use serde::{Deserialize, Serialize};
use std::fmt;
use utoipa::ToSchema;

use super::webhooks::WebhookRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Disconnected,
    Connecting,
    WaitingForQr,
    WaitingForPairCode,
    Connected,
    LoggedIn,
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Disconnected => "disconnected",
            SessionStatus::Connecting => "connecting",
            SessionStatus::WaitingForQr => "waiting_for_qr",
            SessionStatus::WaitingForPairCode => "waiting_for_pair_code",
            SessionStatus::Connected => "connected",
            SessionStatus::LoggedIn => "logged_in",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "connecting" => SessionStatus::Connecting,
            "waiting_for_qr" => SessionStatus::WaitingForQr,
            "waiting_for_pair_code" => SessionStatus::WaitingForPairCode,
            "connected" => SessionStatus::Connected,
            "logged_in" => SessionStatus::LoggedIn,
            _ => SessionStatus::Disconnected,
        }
    }

    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        matches!(self, SessionStatus::LoggedIn | SessionStatus::Connected)
    }

    #[allow(dead_code)]
    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            SessionStatus::Connecting
                | SessionStatus::WaitingForQr
                | SessionStatus::WaitingForPairCode
        )
    }

    #[allow(dead_code)]
    pub fn badge_class(&self) -> &'static str {
        match self {
            SessionStatus::LoggedIn | SessionStatus::Connected => "bg-success",
            SessionStatus::Connecting
            | SessionStatus::WaitingForQr
            | SessionStatus::WaitingForPairCode => "bg-warning",
            SessionStatus::Disconnected => "bg-secondary",
        }
    }
}

/// Session information
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionInfo {
    /// Unique session ID
    pub id: String,
    /// Optional friendly name
    pub name: Option<String>,
    /// Phone number when logged in
    pub phone_number: Option<String>,
    /// WhatsApp display name
    pub push_name: Option<String>,
    /// Current status
    pub status: SessionStatus,
    /// Creation timestamp
    pub created_at: i64,
    /// Last update timestamp
    pub updated_at: i64,
    /// Last successful connection timestamp
    pub last_connected_at: Option<i64>,
    /// Whether session is authenticated
    pub is_logged_in: bool,
    /// `whatsapp_web` (default, unofficial multi-device protocol) or
    /// `whatsapp_cloud` (Meta's official Cloud API, set via
    /// `POST /sessions/{id}/cloud/connect`).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Cloud API WhatsApp Business Account ID, set only on `whatsapp_cloud`
    /// sessions.
    #[serde(default)]
    pub cloud_waba_id: Option<String>,
    /// Cloud API phone number ID, set only on `whatsapp_cloud` sessions.
    #[serde(default)]
    pub cloud_phone_number_id: Option<String>,
    /// Cloud API Meta Business ID, set only on `whatsapp_cloud` sessions.
    #[serde(default)]
    pub cloud_business_id: Option<String>,
    /// Cloud API app ID, set only on `whatsapp_cloud` sessions.
    #[serde(default)]
    pub cloud_app_id: Option<String>,
}

fn default_provider() -> String {
    "whatsapp_web".to_string()
}

/// Request to create a new session
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// Optional custom session ID (auto-generated if not provided)
    #[schema(example = "my-session-1")]
    pub id: Option<String>,
    /// Optional friendly name for the session
    #[schema(example = "Business Account")]
    pub name: Option<String>,
    /// When true and `id` already exists, re-scan the SAME session slot
    /// instead of failing with 409: the existing session keeps its ID,
    /// storage, webhooks, and DB row, and a fresh QR/pair connect is
    /// started on it (same behavior as `POST /sessions/{id}/connect`).
    /// This gives consumers a stable `session_id` across re-pairs
    /// instead of fragmenting state over newly minted IDs. Ignored
    /// webhook/name fields on reuse — update those via their own
    /// endpoints. Has no effect when the ID does not exist yet.
    #[serde(default)]
    pub reuse: Option<bool>,
    /// Optional webhook configuration (session will auto-connect after creation)
    pub webhook: Option<WebhookRequest>,
    /// Optional device props override applied to the auto-spawned QR connect.
    /// Only honored on the first pair — subsequent connects reuse persisted props.
    pub device: Option<DevicePropsRequest>,
}

/// Response after creating a session
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateSessionResponse {
    /// Session information
    pub session: SessionInfo,
}

/// Response with list of sessions
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionListResponse {
    /// List of sessions
    pub sessions: Vec<SessionInfo>,
    /// Total count
    pub total: usize,
}

/// Optional per-session device identity override. Only honored on the
/// FIRST pair (connect/pair endpoints) — subsequent connects reuse the
/// props stored by whatsapp-rust at pairing time.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct DevicePropsRequest {
    /// OS string shown in WhatsApp Linked Devices (e.g. "Windows", "Mac OS X")
    #[schema(example = "Windows")]
    pub os: Option<String>,
    /// Platform: desktop, uwp, chrome, firefox, edge, safari, opera, ie,
    /// ipad, android_phone, android_tablet, ios_phone
    #[schema(example = "desktop")]
    pub platform: Option<String>,
    /// Dotted app version (e.g. "2.3000.1023902713"). Omit to use lib default.
    #[schema(example = "2.3000.1023902713")]
    pub version: Option<String>,
}

/// Request to start QR connect with optional device override
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ConnectRequest {
    /// Optional per-session device props override (first-pair only)
    pub device: Option<DevicePropsRequest>,
}

/// Request to connect with pair code
#[derive(Debug, Deserialize, ToSchema)]
pub struct PairCodeRequest {
    /// Phone number in international format
    #[schema(example = "+1-555-123-4567")]
    pub phone_number: String,
    /// Whether to show push notification on phone
    #[serde(default)]
    pub show_push_notification: bool,
    /// Optional per-session device props override (first-pair only)
    pub device: Option<DevicePropsRequest>,
}

/// Response with pair code
#[derive(Debug, Serialize, ToSchema)]
pub struct PairCodeResponse {
    /// 8-character pairing code
    pub code: String,
    /// Timeout in seconds
    pub timeout_seconds: u64,
}

/// QR code response
#[derive(Debug, Serialize, ToSchema)]
pub struct QrCodeResponse {
    /// QR code data (can be rendered as QR code image)
    pub qr_codes: Vec<String>,
    /// Timeout in seconds before QR code expires
    pub timeout_seconds: u64,
    /// Current session status
    pub status: SessionStatus,
}

/// Pair-flow telemetry — surfaced through /status so the backend can
/// render meaningful progress instead of polling /qr blindly.
#[derive(Debug, Default, Serialize, ToSchema)]
pub struct PairStatus {
    pub last_qr_at: Option<i64>,
    pub last_pair_code_at: Option<i64>,
    pub pair_code_expires_at: Option<i64>,
    pub last_error: Option<String>,
    pub attempts: u32,
}

/// Session status response
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionStatusResponse {
    /// Current status
    pub status: SessionStatus,
    /// Whether logged in
    pub is_logged_in: bool,
    /// Whether the underlying socket is currently connected. Distinct
    /// from `is_logged_in`: a cached `logged_in` status can outlive a
    /// dead socket ("limbo"), and a live socket can precede login
    /// during QR/pair flows.
    pub socket_alive: bool,
    /// Whether the session is paused (`POST /sessions/{id}/pause`): the
    /// client deliberately keeps no connection open until resumed.
    /// `false` when there is no live client in this process.
    pub paused: bool,
    /// Phone number if available
    pub phone_number: Option<String>,
    /// Display name if available
    pub push_name: Option<String>,
    /// Pair flow telemetry (always present, fields may be null)
    pub pair: PairStatus,
    /// Finer-grained connection state from the underlying client's own
    /// `reachability()`, distinct from `status`/`socket_alive`: one of
    /// `reachable`, `reconnecting`, `paused`, `unsupervised` or `finished`.
    /// `null` when there is no live client in this process at all (the
    /// coarse `status`/`is_logged_in` above already cover that case).
    pub reachability: Option<String>,
}

/// Device information
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeviceInfo {
    /// Device ID
    pub device_id: Option<u32>,
    /// Phone number JID
    pub phone_number: Option<String>,
    /// Linked ID
    pub lid: Option<String>,
    /// Push name
    pub push_name: Option<String>,
}
