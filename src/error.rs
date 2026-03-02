use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum ApiError {
    #[error("Not authenticated")]
    NotAuthenticated,

    #[error("Client not connected")]
    NotConnected,

    #[error("Client already connected")]
    AlreadyConnected,

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Invalid JID: {0}")]
    InvalidJid(String),

    #[error("Message not found: {0}")]
    MessageNotFound(String),

    #[error("Contact not found: {0}")]
    ContactNotFound(String),

    #[error("Group not found: {0}")]
    GroupNotFound(String),

    #[error("Media upload failed: {0}")]
    MediaUploadFailed(String),

    #[error("Media download failed: {0}")]
    MediaDownloadFailed(String),

    #[error("Webhook already registered: {0}")]
    WebhookAlreadyExists(String),

    #[error("Webhook not found: {0}")]
    WebhookNotFound(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("Temporary ban: {0}")]
    TemporaryBan(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Session error: {0}")]
    SessionError(String),

    #[error("NATS error: {0}")]
    NatsError(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_message) = match &self {
            ApiError::NotAuthenticated => (StatusCode::UNAUTHORIZED, self.to_string()),
            ApiError::NotConnected => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            ApiError::AlreadyConnected => (StatusCode::CONFLICT, self.to_string()),
            ApiError::SessionNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::InvalidJid(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::MessageNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::ContactNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::GroupNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::MediaUploadFailed(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::MediaDownloadFailed(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            ApiError::WebhookAlreadyExists(_) => (StatusCode::CONFLICT, self.to_string()),
            ApiError::WebhookNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            ApiError::TemporaryBan(_) => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::SessionError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::NatsError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = Json(json!({
            "success": false,
            "error": {
                "code": status.as_u16(),
                "message": error_message
            }
        }));

        (status, body).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::Internal(err.to_string())
    }
}
