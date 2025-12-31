use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Response after uploading media
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UploadMediaResponse {
    /// Media URL for download
    pub url: String,
    /// Direct path on WhatsApp servers
    pub direct_path: String,
    /// Media encryption key (base64)
    pub media_key: String,
    /// File SHA256 hash (base64)
    pub file_sha256: String,
    /// Encrypted file SHA256 hash (base64)
    pub file_enc_sha256: String,
    /// File size in bytes
    pub file_length: u64,
    /// Media type
    pub media_type: MediaType,
    /// MIME type
    pub mimetype: String,
}

/// Media type enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    /// Image (JPEG, PNG, etc.)
    Image,
    /// Video (MP4, etc.)
    Video,
    /// Audio message (voice note)
    Audio,
    /// Document/file
    Document,
    /// Sticker (WebP)
    Sticker,
}

impl MediaType {
    pub fn to_wacore_media_type(&self) -> wacore::download::MediaType {
        match self {
            MediaType::Image => wacore::download::MediaType::Image,
            MediaType::Video => wacore::download::MediaType::Video,
            MediaType::Audio => wacore::download::MediaType::Audio,
            MediaType::Document => wacore::download::MediaType::Document,
            MediaType::Sticker => wacore::download::MediaType::Sticker,
        }
    }
}
