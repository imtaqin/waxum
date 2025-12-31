use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// QR code authentication response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QrCodeResponse {
    /// List of QR code data strings (base64 encoded)
    pub qr_codes: Vec<String>,
    /// Timeout in seconds until QR codes expire
    pub timeout_seconds: u32,
    /// Current connection status
    pub status: ConnectionStatus,
}

/// Request to generate pair code for phone number authentication
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PairCodeRequest {
    /// Phone number in international format (e.g., "559999999999")
    #[schema(example = "559999999999")]
    pub phone_number: String,
}

/// Pair code response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PairCodeResponse {
    /// 8-character pair code to enter on phone
    #[schema(example = "ABCD-EFGH")]
    pub pair_code: String,
    /// Phone number the code was sent to
    pub phone_number: String,
    /// Current connection status
    pub status: ConnectionStatus,
}

/// Connection and authentication status
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StatusResponse {
    /// Current connection status
    pub status: ConnectionStatus,
    /// Whether the client is logged in
    pub logged_in: bool,
    /// Phone number JID if logged in
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    /// User's push name if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_name: Option<String>,
}

/// Connection status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Not connected to WhatsApp servers
    Disconnected,
    /// Establishing connection
    Connecting,
    /// Waiting for QR code scan
    WaitingForQr,
    /// Waiting for pair code entry
    WaitingForPairCode,
    /// Socket connected but not logged in
    Connected,
    /// Fully authenticated and ready
    LoggedIn,
}

impl From<crate::state::ConnectionState> for ConnectionStatus {
    fn from(state: crate::state::ConnectionState) -> Self {
        match state {
            crate::state::ConnectionState::Disconnected => ConnectionStatus::Disconnected,
            crate::state::ConnectionState::Connecting => ConnectionStatus::Connecting,
            crate::state::ConnectionState::WaitingForQr => ConnectionStatus::WaitingForQr,
            crate::state::ConnectionState::WaitingForPairCode => ConnectionStatus::WaitingForPairCode,
            crate::state::ConnectionState::Connected => ConnectionStatus::Connected,
            crate::state::ConnectionState::LoggedIn => ConnectionStatus::LoggedIn,
        }
    }
}
