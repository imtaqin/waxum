use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::shared::messages::models::MediaData;

/// Changes to the linked account's own profile. Every field is optional but
/// at least one must be present; fields left out are not touched.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProfileRequest {
    /// Display name shown to contacts (push name), 1 to 25 characters.
    pub name: Option<String>,

    /// The "About" text, up to 139 characters. An empty string clears it.
    pub about: Option<String>,

    /// Optional emoji shown with the About text. Only used together with
    /// `about`; ignored when `about` is empty.
    pub about_emoji: Option<String>,

    /// New profile picture, as a URL or base64. Any common image format is
    /// accepted; it is centre-cropped to a square and re-encoded as JPEG.
    pub picture: Option<MediaData>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ProfileResponse {
    pub jid: Option<String>,
    pub lid: Option<String>,
    pub phone_number: Option<String>,
    pub name: String,
    pub about: Option<String>,
    /// Direct URL of the full-size profile picture, if one is set.
    pub picture_url: Option<String>,
}
