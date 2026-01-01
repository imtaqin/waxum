use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QrCodeResponse {

    pub qr_codes: Vec<String>,

    pub timeout_seconds: u32,

    pub status: ConnectionStatus,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PairCodeRequest {

    #[schema(example = "559999999999")]
    pub phone_number: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PairCodeResponse {

    #[schema(example = "ABCD-EFGH")]
    pub pair_code: String,

    pub phone_number: String,

    pub status: ConnectionStatus,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StatusResponse {

    pub status: ConnectionStatus,

    pub logged_in: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {

    Disconnected,

    Connecting,

    WaitingForQr,

    WaitingForPairCode,

    Connected,

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
