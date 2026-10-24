//! Edits the linked account's own profile: display name, About text and
//! profile picture. Other contacts' pictures are read through `contacts`.
//!
//! WhatsApp expects a square JPEG (about 640x640), so an uploaded picture is
//! decoded, centre-cropped and re-encoded here rather than pushed as-is. The
//! whole request is validated before anything is sent, so bad input fails the
//! request instead of applying only some of the fields.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::api::common::SuccessResponse;
use crate::api::shared::messages::handlers::get_media_data;
use crate::api::web::profile::models::UpdateProfileRequest;
use crate::error::ApiError;
use crate::state::AppState;

const PICTURE_SIDE: u32 = 640;
const NAME_MAX_CHARS: usize = 25;
const ABOUT_MAX_CHARS: usize = 139;
const EMOJI_MAX_CHARS: usize = 8;

const TEXT_STATUS_DOC: wacore::iq::mex::MexDoc = wacore::iq::mex::MexDoc {
    name: "WAWebMexUpdateTextStatusJobMutation",
    id: "9152604461510864",
};

/// Duration WhatsApp Web sends for an About that never expires (`-1`).
const TEXT_STATUS_NO_EXPIRY: i64 = -1;

/// Sets or clears the About through the text status mutation, which is what
/// WhatsApp Web uses today.
///
/// The older `status` IQ (`set_status_text`) is still accepted and answered
/// by the server, and the new value even reads back through contact lookups,
/// but the phone app ignores it, so About edits made only that way never
/// showed up on the account. Clearing sends a null text with a zero
/// duration, exactly as WhatsApp Web does.
async fn set_about(
    client: &whatsapp_rust::Client,
    about: &str,
    emoji: Option<&str>,
) -> Result<(), ApiError> {
    let about = about.trim();
    let input = if about.is_empty() {
        serde_json::json!({"text": null, "ephemeral_duration_sec": 0})
    } else {
        let mut input = serde_json::json!({
            "text": about,
            "ephemeral_duration_sec": TEXT_STATUS_NO_EXPIRY,
        });
        if let Some(emoji) = emoji.map(str::trim).filter(|e| !e.is_empty()) {
            input["emoji"] = serde_json::json!({"content": emoji});
        }
        input
    };
    let request = whatsapp_rust::MexRequest::new_raw(
        TEXT_STATUS_DOC,
        &[],
        serde_json::json!({"input": input}),
    );
    let response = client
        .mex()
        .execute(request)
        .await
        .map_err(|e| ApiError::Internal(format!("about: {e}")))?;
    if let Some(errors) = response.errors.filter(|e| !e.is_empty()) {
        return Err(ApiError::Internal(format!("about: {}", errors[0].message)));
    }
    Ok(())
}

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_live_client().ok_or(ApiError::NotConnected)
}

pub(crate) fn square_jpeg(bytes: Vec<u8>) -> Result<Vec<u8>, ApiError> {
    use image::{codecs::jpeg::JpegEncoder, imageops::FilterType};

    let decoded = image::load_from_memory(&bytes)
        .map_err(|_| ApiError::BadRequest("picture is not a readable image".into()))?;
    let square = decoded
        .resize_to_fill(PICTURE_SIDE, PICTURE_SIDE, FilterType::Lanczos3)
        .to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&square)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(out)
}

fn validate(request: &UpdateProfileRequest) -> Result<(), ApiError> {
    if request.name.is_none() && request.about.is_none() && request.picture.is_none() {
        return Err(ApiError::BadRequest(
            "send at least one of name, about or picture".into(),
        ));
    }
    if let Some(name) = &request.name {
        let length = name.trim().chars().count();
        if length == 0 || length > NAME_MAX_CHARS {
            return Err(ApiError::BadRequest(format!(
                "name must be 1 to {NAME_MAX_CHARS} characters"
            )));
        }
    }
    if let Some(about) = &request.about {
        if about.chars().count() > ABOUT_MAX_CHARS {
            return Err(ApiError::BadRequest(format!(
                "about must be at most {ABOUT_MAX_CHARS} characters"
            )));
        }
    }
    if let Some(emoji) = &request.about_emoji {
        if request.about.is_none() {
            return Err(ApiError::BadRequest(
                "about_emoji needs about to be set as well".into(),
            ));
        }
        if emoji.trim().chars().count() > EMOJI_MAX_CHARS {
            return Err(ApiError::BadRequest(format!(
                "about_emoji must be at most {EMOJI_MAX_CHARS} characters"
            )));
        }
    }
    Ok(())
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/profile",
    tag = "profile",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = UpdateProfileRequest,
    responses(
        (status = 200, description = "Profile updated", body = SuccessResponse),
        (status = 400, description = "Nothing to change, or an invalid name, about text or picture"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn update_profile(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<UpdateProfileRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    validate(&request)?;

    let picture = match &request.picture {
        Some(media) => {
            let (bytes, _) = get_media_data(media).await?;
            let jpeg = tokio::task::spawn_blocking(move || square_jpeg(bytes))
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))??;
            Some(jpeg)
        }
        None => None,
    };

    let client = get_client(&state, &session_id)?;
    let profile = client.profile();
    let mut updated = Vec::new();

    if let Some(about) = &request.about {
        set_about(&client, about, request.about_emoji.as_deref()).await?;
        if let Err(e) = profile.set_status_text(about).await {
            tracing::warn!(session_id = %session_id, "legacy about update failed: {e}");
        }
        updated.push("about");
    }
    if let Some(name) = &request.name {
        let outcome = profile
            .set_push_name(name.trim())
            .await
            .map_err(|e| ApiError::Internal(format!("name: {e}")))?;
        if let whatsapp_rust::PushNameOutcome::SyncPending { source } = &outcome {
            tracing::warn!(
                session_id = %session_id,
                "push name sent but the app-state sync is still pending: {source}"
            );
        }
        updated.push("name");
    }
    if let Some(jpeg) = picture {
        profile
            .set_profile_picture(jpeg)
            .await
            .map_err(|e| ApiError::Internal(format!("picture: {e}")))?;
        updated.push("picture");
    }

    Ok(Json(SuccessResponse::with_message(format!(
        "Updated {}",
        updated.join(", ")
    ))))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/profile/picture",
    tag = "profile",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Profile picture removed", body = SuccessResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn remove_profile_picture(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    client
        .profile()
        .remove_profile_picture()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(SuccessResponse::with_message(
        "Profile picture removed",
    )))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/profile",
    tag = "profile",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "The linked account's own name, About and picture", body = crate::api::web::profile::models::ProfileResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_profile(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<crate::api::web::profile::models::ProfileResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = client.pn();
    let mut about = None;
    let mut picture_url = None;
    if let Some(own) = jid.clone() {
        if let Ok(info) = client
            .contacts()
            .get_user_info(std::slice::from_ref(&own))
            .await
        {
            about = info.into_values().next().and_then(|i| i.status);
        }
        let request = whatsapp_rust::ProfilePictureRequest::new(
            whatsapp_rust::ProfilePictureTarget::Contact(&own),
            whatsapp_rust::ProfilePictureType::Full,
        );
        if let Ok(lookup) = client.pictures().lookup(request).await {
            if let Some(picture) = lookup.into_found() {
                picture_url = Some(picture.url);
            }
        }
    }
    Ok(Json(crate::api::web::profile::models::ProfileResponse {
        phone_number: jid.as_ref().map(|j| j.user.to_string()),
        jid: jid.map(|j| j.to_string()),
        lid: client.lid().map(|j| j.to_string()),
        name: client.push_name(),
        about,
        picture_url,
    }))
}
