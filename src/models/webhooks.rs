use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookConfig {
    pub url: String,

    pub events: Vec<WebhookEvent>,

    pub secret: Option<String>,

    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEvent {
    All,

    Message,

    Receipt,

    Presence,

    ChatPresence,

    GroupUpdate,

    JoinedGroup,

    QrCode,

    PairCode,

    Connected,

    Disconnected,

    LoggedOut,

    PictureUpdate,

    UserAboutUpdate,

    PushNameUpdate,

    ContactUpdate,

    DeviceListUpdate,

    PinUpdate,

    MuteUpdate,

    ArchiveUpdate,

    MarkChatAsRead,

    UndecryptableMessage,

    ClientOutdated,

    OfflineSyncPreview,

    OfflineSyncCompleted,
}

impl WebhookEvent {
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
            WebhookEvent::PictureUpdate => event == "picture_update",
            WebhookEvent::UserAboutUpdate => event == "user_about_update",
            WebhookEvent::PushNameUpdate => event == "push_name_update",
            WebhookEvent::ContactUpdate => event == "contact_update",
            WebhookEvent::DeviceListUpdate => event == "device_list_update",
            WebhookEvent::PinUpdate => event == "pin_update",
            WebhookEvent::MuteUpdate => event == "mute_update",
            WebhookEvent::ArchiveUpdate => event == "archive_update",
            WebhookEvent::MarkChatAsRead => event == "mark_chat_as_read",
            WebhookEvent::UndecryptableMessage => event == "undecryptable_message",
            WebhookEvent::ClientOutdated => event == "client_outdated",
            WebhookEvent::OfflineSyncPreview => event == "offline_sync_preview",
            WebhookEvent::OfflineSyncCompleted => event == "offline_sync_completed",
        }
    }

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
            WebhookEvent::PictureUpdate => "picture_update",
            WebhookEvent::UserAboutUpdate => "user_about_update",
            WebhookEvent::PushNameUpdate => "push_name_update",
            WebhookEvent::ContactUpdate => "contact_update",
            WebhookEvent::DeviceListUpdate => "device_list_update",
            WebhookEvent::PinUpdate => "pin_update",
            WebhookEvent::MuteUpdate => "mute_update",
            WebhookEvent::ArchiveUpdate => "archive_update",
            WebhookEvent::MarkChatAsRead => "mark_chat_as_read",
            WebhookEvent::UndecryptableMessage => "undecryptable_message",
            WebhookEvent::ClientOutdated => "client_outdated",
            WebhookEvent::OfflineSyncPreview => "offline_sync_preview",
            WebhookEvent::OfflineSyncCompleted => "offline_sync_completed",
        }
    }

    /// Parse from string
    #[allow(dead_code)]
    #[allow(clippy::should_implement_trait)]
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
            "picture_update" => Some(WebhookEvent::PictureUpdate),
            "user_about_update" => Some(WebhookEvent::UserAboutUpdate),
            "push_name_update" => Some(WebhookEvent::PushNameUpdate),
            "contact_update" => Some(WebhookEvent::ContactUpdate),
            "device_list_update" => Some(WebhookEvent::DeviceListUpdate),
            "pin_update" => Some(WebhookEvent::PinUpdate),
            "mute_update" => Some(WebhookEvent::MuteUpdate),
            "archive_update" => Some(WebhookEvent::ArchiveUpdate),
            "mark_chat_as_read" => Some(WebhookEvent::MarkChatAsRead),
            "undecryptable_message" => Some(WebhookEvent::UndecryptableMessage),
            "client_outdated" => Some(WebhookEvent::ClientOutdated),
            "offline_sync_preview" => Some(WebhookEvent::OfflineSyncPreview),
            "offline_sync_completed" => Some(WebhookEvent::OfflineSyncCompleted),
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

/// A webhook config bundled with its runtime ID so clients can call the
/// `DELETE /webhooks/{webhook_id}` endpoint.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WebhookConfigWithId {
    pub id: String,
    pub url: String,
    pub events: Vec<WebhookEvent>,
    pub secret: Option<String>,
    pub enabled: bool,
}

impl From<(String, WebhookConfig)> for WebhookConfigWithId {
    fn from((id, cfg): (String, WebhookConfig)) -> Self {
        Self {
            id,
            url: cfg.url,
            events: cfg.events,
            secret: cfg.secret,
            enabled: cfg.enabled,
        }
    }
}

/// Response with list of webhooks
#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookListResponse {
    /// List of webhooks, each with its ID so clients can DELETE by ID.
    pub webhooks: Vec<WebhookConfigWithId>,
    /// Total count
    pub count: usize,
}
