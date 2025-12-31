use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// Note: reply_to fields are included in request structs for API schema completeness
// but not yet fully implemented in handlers

/// Request to send a text message
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SendTextRequest {
    /// Recipient JID (phone number or group ID)
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Message text content
    #[schema(example = "Hello, World!")]
    pub text: String,
    /// Optional message to reply to
    pub reply_to: Option<String>,
}

/// Request to send an image message
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendImageRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Image URL or base64 data
    pub image: MediaData,
    /// Optional caption
    pub caption: Option<String>,
    /// Optional message to reply to
    pub reply_to: Option<String>,
}

/// Request to send a video message
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendVideoRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Video URL or base64 data
    pub video: MediaData,
    /// Optional caption
    pub caption: Option<String>,
    /// Optional message to reply to
    pub reply_to: Option<String>,
}

/// Request to send an audio message
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendAudioRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Audio URL or base64 data
    pub audio: MediaData,
    /// Whether this is a voice note (ptt)
    #[serde(default)]
    pub ptt: bool,
    /// Optional message to reply to
    pub reply_to: Option<String>,
}

/// Request to send a document message
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendDocumentRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Document URL or base64 data
    pub document: MediaData,
    /// File name
    #[schema(example = "document.pdf")]
    pub filename: String,
    /// Optional caption
    pub caption: Option<String>,
    /// Optional message to reply to
    pub reply_to: Option<String>,
}

/// Request to send a sticker message
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendStickerRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Sticker URL or base64 data (WebP format)
    pub sticker: MediaData,
    /// Optional message to reply to
    pub reply_to: Option<String>,
}

/// Request to send a location message
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendLocationRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Latitude
    #[schema(example = -23.5505)]
    pub latitude: f64,
    /// Longitude
    #[schema(example = -46.6333)]
    pub longitude: f64,
    /// Optional location name
    pub name: Option<String>,
    /// Optional address
    pub address: Option<String>,
    /// Optional message to reply to
    pub reply_to: Option<String>,
}

/// Request to send a contact message
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendContactRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Contact vCard data
    pub contact: ContactCard,
    /// Optional message to reply to
    pub reply_to: Option<String>,
}

/// Contact card information
#[derive(Debug, Deserialize, ToSchema)]
pub struct ContactCard {
    /// Display name
    pub display_name: String,
    /// Phone numbers
    pub phones: Vec<ContactPhone>,
    /// Optional organization
    pub organization: Option<String>,
}

/// Contact phone entry
#[derive(Debug, Deserialize, ToSchema)]
pub struct ContactPhone {
    /// Phone number
    pub number: String,
    /// Phone type (CELL, HOME, WORK)
    #[serde(default = "default_phone_type")]
    pub phone_type: String,
}

fn default_phone_type() -> String {
    "CELL".to_string()
}

/// Request to edit a message
#[derive(Debug, Deserialize, ToSchema)]
pub struct EditMessageRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Original message ID
    pub message_id: String,
    /// New text content
    pub text: String,
}

/// Request to send a reaction
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendReactionRequest {
    /// Chat JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Message ID to react to
    pub message_id: String,
    /// Emoji reaction (empty string to remove)
    #[schema(example = "👍")]
    pub emoji: String,
}

/// Media data - either URL or base64
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum MediaData {
    /// URL to fetch media from
    Url { url: String },
    /// Base64 encoded media data
    Base64 { data: String, mimetype: String },
    /// Pre-uploaded media reference
    Uploaded {
        url: String,
        direct_path: String,
        media_key: String,
        file_sha256: String,
        file_enc_sha256: String,
        file_length: u64,
        mimetype: String,
    },
}

/// Response after sending a message
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MessageResponse {
    /// Generated message ID
    pub message_id: String,
    /// Timestamp when the message was sent
    pub timestamp: i64,
    /// Recipient JID
    pub to: String,
}

/// Legacy request format (kept for compatibility)
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendMessageRequest {
    /// Recipient JID (phone number or group ID)
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Message text content
    #[schema(example = "Hello, World!")]
    pub text: String,
}
