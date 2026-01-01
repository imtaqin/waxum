use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SendTextRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = "Hello, World!")]
    pub text: String,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendImageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub image: MediaData,

    pub caption: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendVideoRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub video: MediaData,

    pub caption: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendAudioRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub audio: MediaData,

    #[serde(default)]
    pub ptt: bool,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendDocumentRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub document: MediaData,

    #[schema(example = "document.pdf")]
    pub filename: String,

    pub caption: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendStickerRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub sticker: MediaData,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendLocationRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = -23.5505)]
    pub latitude: f64,

    #[schema(example = -46.6333)]
    pub longitude: f64,

    pub name: Option<String>,

    pub address: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendContactRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub contact: ContactCard,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ContactCard {
    pub display_name: String,

    pub phones: Vec<ContactPhone>,

    pub organization: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ContactPhone {
    pub number: String,

    #[serde(default = "default_phone_type")]
    pub phone_type: String,
}

fn default_phone_type() -> String {
    "CELL".to_string()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EditMessageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub message_id: String,

    pub text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendReactionRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub message_id: String,

    #[schema(example = "👍")]
    pub emoji: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum MediaData {
    Url {
        url: String,
    },

    Base64 {
        data: String,
        mimetype: String,
    },

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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MessageResponse {
    pub message_id: String,

    pub timestamp: i64,

    pub to: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendMessageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = "Hello, World!")]
    pub text: String,
}
