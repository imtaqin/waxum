use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UploadMediaResponse {
    pub url: String,

    pub direct_path: String,

    pub media_key: String,

    pub file_sha256: String,

    pub file_enc_sha256: String,

    pub file_length: u64,

    pub media_type: MediaType,

    pub mimetype: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,

    Video,

    Audio,

    Document,

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
