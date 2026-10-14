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

    /// New profile picture, as a URL or base64. Any common image format is
    /// accepted; it is centre-cropped to a square and re-encoded as JPEG.
    pub picture: Option<MediaData>,
}
