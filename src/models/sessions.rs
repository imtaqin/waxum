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
    /// Optional webhook configuration (session will auto-connect after creation)
    pub webhook: Option<WebhookRequest>,
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

/// Request to connect with pair code
#[derive(Debug, Deserialize, ToSchema)]
pub struct PairCodeRequest {
    /// Phone number in international format
    #[schema(example = "+1-555-123-4567")]
    pub phone_number: String,
    /// Whether to show push notification on phone
    #[serde(default)]
    pub show_push_notification: bool,
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

/// Session status response
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionStatusResponse {
    /// Current status
    pub status: SessionStatus,
    /// Whether logged in
    pub is_logged_in: bool,
    /// Phone number if available
    pub phone_number: Option<String>,
    /// Display name if available
    pub push_name: Option<String>,
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
