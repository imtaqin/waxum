use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Webhook configuration
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookConfig {
    /// Webhook URL to call
    pub url: String,
    /// Events to subscribe to
    pub events: Vec<WebhookEvent>,
    /// Optional secret for HMAC signature
    pub secret: Option<String>,
    /// Whether webhook is enabled
    pub enabled: bool,
}

/// Webhook event types
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEvent {
    /// All events
    All,
    /// New message received
    Message,
    /// Message receipt (delivered, read, etc.)
    Receipt,
    /// Presence update
    Presence,
    /// Chat presence (typing, recording)
    ChatPresence,
    /// Group info update
    GroupUpdate,
    /// Joined a group
    JoinedGroup,
    /// QR code event
    QrCode,
    /// Pair code event
    PairCode,
    /// Connected
    Connected,
    /// Disconnected
    Disconnected,
    /// Logged out
    LoggedOut,
}

impl WebhookEvent {
    /// Check if this event filter matches the given event type
    pub fn matches(&self, event: &str) -> bool {
        match self {
            WebhookEvent::All => true,
            WebhookEvent::Message => event == "message",
            WebhookEvent::Receipt => event == "receipt",
            WebhookEvent::Presence => event == "presence",
            WebhookEvent::ChatPresence => event == "chat_presence",
            WebhookEvent::GroupUpdate => event == "group_update",
            WebhookEvent::JoinedGroup => event == "joined_group",
            WebhookEvent::QrCode => event == "qr_code",
            WebhookEvent::PairCode => event == "pair_code",
            WebhookEvent::Connected => event == "connected",
            WebhookEvent::Disconnected => event == "disconnected",
            WebhookEvent::LoggedOut => event == "logged_out",
        }
    }

    /// Convert to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            WebhookEvent::All => "all",
            WebhookEvent::Message => "message",
            WebhookEvent::Receipt => "receipt",
            WebhookEvent::Presence => "presence",
            WebhookEvent::ChatPresence => "chat_presence",
            WebhookEvent::GroupUpdate => "group_update",
            WebhookEvent::JoinedGroup => "joined_group",
            WebhookEvent::QrCode => "qr_code",
            WebhookEvent::PairCode => "pair_code",
            WebhookEvent::Connected => "connected",
            WebhookEvent::Disconnected => "disconnected",
            WebhookEvent::LoggedOut => "logged_out",
        }
    }

    /// Parse from string
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "all" => Some(WebhookEvent::All),
            "message" => Some(WebhookEvent::Message),
            "receipt" => Some(WebhookEvent::Receipt),
            "presence" => Some(WebhookEvent::Presence),
            "chat_presence" => Some(WebhookEvent::ChatPresence),
            "group_update" => Some(WebhookEvent::GroupUpdate),
            "joined_group" => Some(WebhookEvent::JoinedGroup),
            "qr_code" => Some(WebhookEvent::QrCode),
            "pair_code" => Some(WebhookEvent::PairCode),
            "connected" => Some(WebhookEvent::Connected),
            "disconnected" => Some(WebhookEvent::Disconnected),
            "logged_out" => Some(WebhookEvent::LoggedOut),
            _ => None,
        }
    }
}

/// Request to register a webhook
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterWebhookRequest {
    /// Webhook URL
    #[schema(example = "https://example.com/webhook")]
    pub url: String,
    /// Events to subscribe to
    pub events: Vec<WebhookEvent>,
    /// Optional secret for signature verification
    pub secret: Option<String>,
}

/// Webhook configuration for session creation
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WebhookRequest {
    /// Webhook URL
    #[schema(example = "https://example.com/webhook")]
    pub url: String,
    /// Events to subscribe to (defaults to all if not provided)
    #[serde(default = "default_events")]
    pub events: Vec<WebhookEvent>,
    /// Optional secret for HMAC signature verification
    pub secret: Option<String>,
}

fn default_events() -> Vec<WebhookEvent> {
    vec![WebhookEvent::All]
}

/// Response with list of webhooks
#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookListResponse {
    /// List of webhooks
    pub webhooks: Vec<WebhookConfig>,
    /// Total count
    pub count: usize,
}
