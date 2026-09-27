//! WhatsApp Cloud API account administration and BSP routes for
//! `whatsapp_cloud` sessions: phone number registration/verification,
//! business profile, typing indicator, webhook subscriptions, templates,
//! QR codes, blocked users, India business compliance, analytics and
//! billing, Payments API messages, Flow endpoint metrics, and the
//! Embedded Signup system-user and credit-line-sharing steps.
//!
//! Each route is a thin pass-through to one Graph API call (see
//! [`crate::cloud::graph`]) that returns Meta's JSON unchanged, so no
//! response shapes are re-modeled here. WABA-, business- and app-scoped
//! calls use the IDs stored by `/cloud/connect`; a session connected
//! without `business_id` gets a `400` naming the missing field rather
//! than a Graph API error.
//!
//! A `4xx` from Meta comes back as waxum's `400` with Meta's error body,
//! since it almost always means the request itself needs fixing (bad
//! template component, unknown QR code, PIN mismatch). A Meta `5xx` or a
//! transport failure is a `500`.
//!
//! IDs taken from the request path (template, QR code, Flow, credit line,
//! allocation config) are checked with [`crate::cloud::graph::graph_id`]
//! before they're placed in a Graph API URL.

use axum::{
    extract::{Multipart, Path, Query, State},
    Json,
};
use reqwest::Method;
use serde_json::{json, Value};

use crate::cloud::client::{CloudClient, CloudError};
use crate::cloud::graph::graph_id;
use crate::error::ApiError;
use crate::models::cloud_admin::*;
use crate::models::messages::MessageResponse;
use crate::models::sessions::SessionInfo;
use crate::state::AppState;

struct Ctx {
    client: CloudClient,
    info: SessionInfo,
}

impl Ctx {
    fn stored(&self, value: &Option<String>, field: &str) -> Result<String, ApiError> {
        value
            .as_deref()
            .and_then(graph_id)
            .map(str::to_string)
            .ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "session has no {field}; set it via POST /cloud/connect"
                ))
            })
    }

    fn waba(&self) -> Result<String, ApiError> {
        self.stored(&self.info.cloud_waba_id, "waba_id")
    }

    fn business(&self) -> Result<String, ApiError> {
        self.stored(&self.info.cloud_business_id, "business_id")
    }

    fn app(&self) -> Result<String, ApiError> {
        self.stored(&self.info.cloud_app_id, "app_id")
    }

    fn phone(&self) -> String {
        self.client.phone_number_id().to_string()
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Json<Value>, ApiError> {
        self.client
            .graph(method, path, query, body)
            .await
            .map(Json)
            .map_err(meta)
    }
}

async fn ctx(state: &AppState, session_id: &str) -> Result<Ctx, ApiError> {
    let manager = state.session_manager();
    let creds = manager
        .get_cloud_credentials(session_id)
        .await?
        .ok_or_else(|| {
            ApiError::BadRequest("only supported for whatsapp_cloud sessions".to_string())
        })?;
    let info = manager
        .get_session(session_id)
        .await?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))?;
    Ok(Ctx {
        client: CloudClient::new(&creds.phone_number_id, &creds.access_token),
        info,
    })
}

fn meta(e: CloudError) -> ApiError {
    match e {
        CloudError::Api { status, body } if (400..500).contains(&status) => {
            ApiError::BadRequest(format!("Meta rejected the request ({status}): {body}"))
        }
        other => ApiError::Internal(format!("Meta call failed: {other}")),
    }
}

fn path_id(raw: &str, what: &str) -> Result<String, ApiError> {
    graph_id(raw)
        .map(str::to_string)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid {what}")))
}

fn one_of(value: &str, allowed: &[&str], field: &str) -> Result<(), ApiError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "{field} must be one of {}",
            allowed.join(", ")
        )))
    }
}

/// Turns a comma-separated query value into the `["a","b"]` array literal
/// Graph API field expansion expects, refusing characters that could
/// break out of the expansion (`)`, `.`, quotes).
fn field_list(raw: &str, field: &str) -> Result<String, ApiError> {
    let items: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if items.iter().any(|s| {
        !s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+'))
    }) {
        return Err(ApiError::BadRequest(format!(
            "{field} may only contain letters, digits, '_' and '+'"
        )));
    }
    Ok(serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string()))
}

fn sent(resp: &Value, to: String) -> Json<MessageResponse> {
    Json(MessageResponse {
        message_id: resp
            .pointer("/messages/0/id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to,
    })
}

fn is_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/waba",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "The session's WhatsApp Business Account (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn get_waba(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    c.call(Method::GET, &c.waba()?, &[], None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/owned-wabas",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "WABAs owned by the session's business"),
        (status = 400, description = "Not a whatsapp_cloud session, or no business_id stored")
    )
)]
pub async fn owned_wabas(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/owned_whatsapp_business_accounts", c.business()?);
    c.call(Method::GET, &path, &[], None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/client-wabas",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Client WABAs shared with the session's business"),
        (status = 400, description = "Not a whatsapp_cloud session, or no business_id stored")
    )
)]
pub async fn client_wabas(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/client_whatsapp_business_accounts", c.business()?);
    c.call(Method::GET, &path, &[], None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/business-portfolio",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "The business portfolio's id, name and timezone"),
        (status = 400, description = "Not a whatsapp_cloud session, or no business_id stored")
    )
)]
pub async fn business_portfolio(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let query = [("fields", "id,name,timezone_id".to_string())];
    c.call(Method::GET, &c.business()?, &query, None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/phone-numbers",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Phone numbers on the session's WABA"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn list_phone_numbers(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let query = [(
        "fields",
        "id,display_phone_number,verified_name,quality_rating,code_verification_status,name_status,platform_type,throughput,is_official_business_account".to_string(),
    )];
    let path = format!("{}/phone_numbers", c.waba()?);
    c.call(Method::GET, &path, &query, None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/phone-number",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "The session's phone number, incl. display name status and messaging limit"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn get_phone_number(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let query = [(
        "fields",
        "id,display_phone_number,verified_name,quality_rating,code_verification_status,name_status,new_name_status,platform_type,throughput,messaging_limit_tier,account_mode".to_string(),
    )];
    c.call(Method::GET, &c.phone(), &query, None).await
}

fn check_pin(pin: &str) -> Result<(), ApiError> {
    if pin.len() == 6 && pin.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "pin must be exactly 6 digits".to_string(),
        ))
    }
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/register",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = RegisterPhoneRequest,
    responses(
        (status = 200, description = "Number registered for Cloud API use"),
        (status = 400, description = "Invalid PIN, or rejected by Meta")
    )
)]
pub async fn register_phone(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RegisterPhoneRequest>,
) -> Result<Json<Value>, ApiError> {
    check_pin(&request.pin)?;
    let c = ctx(&state, &session_id).await?;
    let mut body = json!({ "messaging_product": "whatsapp", "pin": request.pin });
    if let Some(region) = request.data_localization_region {
        body["data_localization_region"] = json!(region);
    }
    if let Some(backup) = request.backup {
        body["backup"] = json!({ "data": backup.data, "password": backup.password });
    }
    let path = format!("{}/register", c.phone());
    c.call(Method::POST, &path, &[], Some(body)).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/deregister",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Number deregistered from Cloud API"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn deregister_phone(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/deregister", c.phone());
    c.call(Method::POST, &path, &[], None).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/request-code",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = RequestCodeRequest,
    responses(
        (status = 200, description = "Verification code sent by SMS or voice"),
        (status = 400, description = "Invalid code_method, or rejected by Meta")
    )
)]
pub async fn request_code(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RequestCodeRequest>,
) -> Result<Json<Value>, ApiError> {
    one_of(&request.code_method, &["SMS", "VOICE"], "code_method")?;
    let c = ctx(&state, &session_id).await?;
    let body = json!({
        "code_method": request.code_method,
        "locale": request.locale.unwrap_or_else(|| "en_US".to_string()),
    });
    let path = format!("{}/request_code", c.phone());
    c.call(Method::POST, &path, &[], Some(body)).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/verify-code",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = VerifyCodeRequest,
    responses(
        (status = 200, description = "Number verified"),
        (status = 400, description = "Wrong code, or rejected by Meta")
    )
)]
pub async fn verify_code(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<VerifyCodeRequest>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/verify_code", c.phone());
    c.call(
        Method::POST,
        &path,
        &[],
        Some(json!({ "code": request.code })),
    )
    .await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/two-step-pin",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = TwoStepPinRequest,
    responses(
        (status = 200, description = "Two-step verification PIN set"),
        (status = 400, description = "Invalid PIN, or rejected by Meta")
    )
)]
pub async fn set_two_step_pin(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<TwoStepPinRequest>,
) -> Result<Json<Value>, ApiError> {
    check_pin(&request.pin)?;
    let c = ctx(&state, &session_id).await?;
    c.call(
        Method::POST,
        &c.phone(),
        &[],
        Some(json!({ "pin": request.pin })),
    )
    .await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/debug-token",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Scopes, expiry and granular WABA permissions of the session's access token (the token itself is not returned)"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn debug_token(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let token = state
        .session_manager()
        .get_cloud_credentials(&session_id)
        .await?
        .map(|creds| creds.access_token)
        .unwrap_or_default();
    c.call(Method::GET, "debug_token", &[("input_token", token)], None)
        .await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/business-profile",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "The number's business profile"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn get_business_profile(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let query = [(
        "fields",
        "about,address,description,email,profile_picture_url,websites,vertical".to_string(),
    )];
    let path = format!("{}/whatsapp_business_profile", c.phone());
    c.call(Method::GET, &path, &query, None).await
}

fn profile_body(r: UpdateBusinessProfileRequest) -> Result<Value, ApiError> {
    let mut body = json!({ "messaging_product": "whatsapp" });
    let fields = [
        ("about", r.about),
        ("address", r.address),
        ("description", r.description),
        ("email", r.email),
        ("vertical", r.vertical),
        ("profile_picture_handle", r.profile_picture_handle),
    ];
    let mut any = false;
    for (key, value) in fields {
        if let Some(v) = value {
            body[key] = json!(v);
            any = true;
        }
    }
    if let Some(websites) = r.websites {
        if websites.len() > 2 {
            return Err(ApiError::BadRequest(
                "at most 2 websites are allowed".to_string(),
            ));
        }
        body["websites"] = json!(websites);
        any = true;
    }
    if !any {
        return Err(ApiError::BadRequest(
            "at least one profile field is required".to_string(),
        ));
    }
    Ok(body)
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/business-profile",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = UpdateBusinessProfileRequest,
    responses(
        (status = 200, description = "Business profile updated"),
        (status = 400, description = "No field given, too many websites, or rejected by Meta")
    )
)]
pub async fn update_business_profile(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<UpdateBusinessProfileRequest>,
) -> Result<Json<Value>, ApiError> {
    let body = profile_body(request)?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/whatsapp_business_profile", c.phone());
    c.call(Method::POST, &path, &[], Some(body)).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/business-profile/photo",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Photo uploaded via the Resumable Upload API and set as the profile picture (multipart field `file`, JPEG or PNG)"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn upload_business_profile_photo(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let app_id = c.app()?;

    let mut file: Option<(Vec<u8>, String, String)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        if field.name() == Some("file") {
            let mime = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();
            let name = field.file_name().unwrap_or("profile.jpg").to_string();
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?
                .to_vec();
            file = Some((bytes, mime, name));
        }
    }
    let (bytes, mime, name) =
        file.ok_or_else(|| ApiError::BadRequest("No file provided".to_string()))?;
    one_of(&mime, &["image/jpeg", "image/png"], "file content type")?;

    let handle = c
        .client
        .resumable_upload(&app_id, bytes, &mime, &name)
        .await
        .map_err(meta)?;
    let body = json!({ "messaging_product": "whatsapp", "profile_picture_handle": handle });
    let path = format!("{}/whatsapp_business_profile", c.phone());
    let Json(meta_response) = c.call(Method::POST, &path, &[], Some(body)).await?;
    Ok(Json(
        json!({ "handle": handle, "meta_response": meta_response }),
    ))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/typing",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = TypingIndicatorRequest,
    responses(
        (status = 200, description = "Typing indicator shown and message marked read"),
        (status = 400, description = "Not a whatsapp_cloud session, or rejected by Meta")
    )
)]
pub async fn send_typing(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<TypingIndicatorRequest>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let body = json!({
        "messaging_product": "whatsapp",
        "status": "read",
        "message_id": request.message_id,
        "typing_indicator": { "type": "text" },
    });
    let path = format!("{}/messages", c.phone());
    c.call(Method::POST, &path, &[], Some(body)).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/subscribed-apps",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Apps subscribed to the session's WABA webhooks"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn list_subscribed_apps(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/subscribed_apps", c.waba()?);
    c.call(Method::GET, &path, &[], None).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/subscribed-apps",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = SubscribeAppRequest,
    responses(
        (status = 200, description = "App subscribed; with both fields set, WABA webhooks go to override_callback_uri"),
        (status = 400, description = "Only one of the override fields given, or rejected by Meta")
    )
)]
pub async fn subscribe_app(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SubscribeAppRequest>,
) -> Result<Json<Value>, ApiError> {
    let body = match (request.override_callback_uri, request.verify_token) {
        (None, None) => None,
        (Some(uri), Some(token)) => {
            crate::net_guard::validate_public_url(&uri)
                .await
                .map_err(ApiError::BadRequest)?;
            Some(json!({ "override_callback_uri": uri, "verify_token": token }))
        }
        _ => {
            return Err(ApiError::BadRequest(
                "override_callback_uri and verify_token must be given together".to_string(),
            ))
        }
    };
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/subscribed_apps", c.waba()?);
    c.call(Method::POST, &path, &[], body).await
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/subscribed-apps",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "App unsubscribed from the session's WABA webhooks"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn unsubscribe_app(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/subscribed_apps", c.waba()?);
    c.call(Method::DELETE, &path, &[], None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/templates",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID"), ListTemplatesQuery),
    responses(
        (status = 200, description = "Message templates on the WABA, paged"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn list_templates(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<ListTemplatesQuery>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let mut query: Vec<(&str, String)> = Vec::new();
    let optional = [
        ("name", q.name),
        ("status", q.status),
        ("category", q.category),
        ("language", q.language),
        ("after", q.after),
        ("before", q.before),
    ];
    for (key, value) in optional {
        if let Some(v) = value {
            query.push((key, v));
        }
    }
    if let Some(limit) = q.limit {
        query.push(("limit", limit.min(1000).to_string()));
    }
    let path = format!("{}/message_templates", c.waba()?);
    c.call(Method::GET, &path, &query, None).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/templates",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = CreateTemplateRequest,
    responses(
        (status = 200, description = "Template submitted for review (id, status, category)"),
        (status = 400, description = "Invalid category/components, or rejected by Meta")
    )
)]
pub async fn create_template(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<CreateTemplateRequest>,
) -> Result<Json<Value>, ApiError> {
    one_of(
        &r.category,
        &["MARKETING", "UTILITY", "AUTHENTICATION"],
        "category",
    )?;
    if !r.components.is_array() {
        return Err(ApiError::BadRequest(
            "components must be an array".to_string(),
        ));
    }
    let c = ctx(&state, &session_id).await?;
    let mut body = json!({
        "name": r.name,
        "language": r.language,
        "category": r.category,
        "components": r.components,
    });
    if let Some(v) = r.allow_category_change {
        body["allow_category_change"] = json!(v);
    }
    if let Some(v) = r.parameter_format {
        body["parameter_format"] = json!(v);
    }
    if let Some(v) = r.message_send_ttl_seconds {
        body["message_send_ttl_seconds"] = json!(v);
    }
    let path = format!("{}/message_templates", c.waba()?);
    c.call(Method::POST, &path, &[], Some(body)).await
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/templates",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID"), DeleteTemplateQuery),
    responses(
        (status = 200, description = "Template deleted (every language, or one when hsm_id is given)"),
        (status = 400, description = "Rejected by Meta")
    )
)]
pub async fn delete_template(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<DeleteTemplateQuery>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let mut query = vec![("name", q.name)];
    if let Some(hsm_id) = q.hsm_id {
        query.push(("hsm_id", hsm_id));
    }
    let path = format!("{}/message_templates", c.waba()?);
    c.call(Method::DELETE, &path, &query, None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/templates/namespace",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "The WABA's message template namespace"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn template_namespace(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let query = [("fields", "message_template_namespace".to_string())];
    c.call(Method::GET, &c.waba()?, &query, None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/templates/{template_id}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("template_id" = String, Path, description = "Template ID")
    ),
    responses(
        (status = 200, description = "The template"),
        (status = 400, description = "Invalid template_id, or rejected by Meta")
    )
)]
pub async fn get_template(
    State(state): State<AppState>,
    Path((session_id, template_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let id = path_id(&template_id, "template_id")?;
    let c = ctx(&state, &session_id).await?;
    c.call(Method::GET, &id, &[], None).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/templates/{template_id}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("template_id" = String, Path, description = "Template ID")
    ),
    request_body = EditTemplateRequest,
    responses(
        (status = 200, description = "Template edited and re-submitted for review"),
        (status = 400, description = "Nothing to edit, or rejected by Meta")
    )
)]
pub async fn edit_template(
    State(state): State<AppState>,
    Path((session_id, template_id)): Path<(String, String)>,
    Json(r): Json<EditTemplateRequest>,
) -> Result<Json<Value>, ApiError> {
    let id = path_id(&template_id, "template_id")?;
    let mut body = json!({});
    if let Some(v) = r.category {
        body["category"] = json!(v);
    }
    if let Some(v) = r.components {
        body["components"] = v;
    }
    if let Some(v) = r.message_send_ttl_seconds {
        body["message_send_ttl_seconds"] = json!(v);
    }
    if body.as_object().is_some_and(|o| o.is_empty()) {
        return Err(ApiError::BadRequest(
            "at least one of category, components, message_send_ttl_seconds is required"
                .to_string(),
        ));
    }
    let c = ctx(&state, &session_id).await?;
    c.call(Method::POST, &id, &[], Some(body)).await
}

fn qr_fields(format: Option<&str>) -> Result<String, ApiError> {
    match format {
        None => Ok("code,prefilled_message,deep_link_url".to_string()),
        Some(f) => {
            one_of(f, &["SVG", "PNG"], "format")?;
            Ok(format!(
                "code,prefilled_message,deep_link_url,qr_image_url.format({f})"
            ))
        }
    }
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/qr-codes",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID"), QrCodeQuery),
    responses(
        (status = 200, description = "The number's QR codes, with image URLs when format is set"),
        (status = 400, description = "Invalid format, or rejected by Meta")
    )
)]
pub async fn list_qr_codes(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<QrCodeQuery>,
) -> Result<Json<Value>, ApiError> {
    let fields = qr_fields(q.format.as_deref())?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/message_qrdls", c.phone());
    c.call(Method::GET, &path, &[("fields", fields)], None)
        .await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/qr-codes",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = CreateQrCodeRequest,
    responses(
        (status = 200, description = "QR code created (code, deep link, image URL)"),
        (status = 400, description = "Invalid image format, or rejected by Meta")
    )
)]
pub async fn create_qr_code(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<CreateQrCodeRequest>,
) -> Result<Json<Value>, ApiError> {
    let image = r.generate_qr_image.unwrap_or_else(|| "SVG".to_string());
    one_of(&image, &["SVG", "PNG"], "generate_qr_image")?;
    let c = ctx(&state, &session_id).await?;
    let body = json!({ "prefilled_message": r.prefilled_message, "generate_qr_image": image });
    let path = format!("{}/message_qrdls", c.phone());
    c.call(Method::POST, &path, &[], Some(body)).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/qr-codes/{code}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("code" = String, Path, description = "QR code"),
        QrCodeQuery
    ),
    responses(
        (status = 200, description = "The QR code"),
        (status = 400, description = "Invalid code or format, or rejected by Meta")
    )
)]
pub async fn get_qr_code(
    State(state): State<AppState>,
    Path((session_id, code)): Path<(String, String)>,
    Query(q): Query<QrCodeQuery>,
) -> Result<Json<Value>, ApiError> {
    let code = path_id(&code, "code")?;
    let fields = qr_fields(q.format.as_deref())?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/message_qrdls", c.phone());
    c.call(
        Method::GET,
        &path,
        &[("fields", fields), ("code", code)],
        None,
    )
    .await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/qr-codes/{code}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("code" = String, Path, description = "QR code")
    ),
    request_body = UpdateQrCodeRequest,
    responses(
        (status = 200, description = "QR code's prefilled message updated"),
        (status = 400, description = "Invalid code, or rejected by Meta")
    )
)]
pub async fn update_qr_code(
    State(state): State<AppState>,
    Path((session_id, code)): Path<(String, String)>,
    Json(r): Json<UpdateQrCodeRequest>,
) -> Result<Json<Value>, ApiError> {
    let code = path_id(&code, "code")?;
    let c = ctx(&state, &session_id).await?;
    let body = json!({ "prefilled_message": r.prefilled_message, "code": code });
    let path = format!("{}/message_qrdls", c.phone());
    c.call(Method::POST, &path, &[], Some(body)).await
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/qr-codes/{code}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("code" = String, Path, description = "QR code")
    ),
    responses(
        (status = 200, description = "QR code deleted"),
        (status = 400, description = "Invalid code, or rejected by Meta")
    )
)]
pub async fn delete_qr_code(
    State(state): State<AppState>,
    Path((session_id, code)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let code = path_id(&code, "code")?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/message_qrdls/{}", c.phone(), code);
    c.call(Method::DELETE, &path, &[], None).await
}

fn block_body(users: Vec<String>) -> Result<Value, ApiError> {
    if users.is_empty() {
        return Err(ApiError::BadRequest("users must not be empty".to_string()));
    }
    Ok(json!({
        "messaging_product": "whatsapp",
        "block_users": users.into_iter().map(|u| json!({ "user": u })).collect::<Vec<_>>(),
    }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/blocked-users",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Users blocked by the number, paged"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn list_blocked_users(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/block_users", c.phone());
    c.call(Method::GET, &path, &[], None).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/blocked-users",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = BlockUsersRequest,
    responses(
        (status = 200, description = "Per-user block results"),
        (status = 400, description = "Empty list, or rejected by Meta")
    )
)]
pub async fn block_users(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<BlockUsersRequest>,
) -> Result<Json<Value>, ApiError> {
    let body = block_body(r.users)?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/block_users", c.phone());
    c.call(Method::POST, &path, &[], Some(body)).await
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/blocked-users",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = BlockUsersRequest,
    responses(
        (status = 200, description = "Per-user unblock results"),
        (status = 400, description = "Empty list, or rejected by Meta")
    )
)]
pub async fn unblock_users(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<BlockUsersRequest>,
) -> Result<Json<Value>, ApiError> {
    let body = block_body(r.users)?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/block_users", c.phone());
    c.call(Method::DELETE, &path, &[], Some(body)).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/business-compliance",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "India business compliance info on the number"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn get_business_compliance(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/business_compliance_info", c.phone());
    c.call(Method::GET, &path, &[], None).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/business-compliance",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = BusinessComplianceRequest,
    responses(
        (status = 200, description = "Compliance info saved"),
        (status = 400, description = "Rejected by Meta")
    )
)]
pub async fn set_business_compliance(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<BusinessComplianceRequest>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let mut body = json!({
        "messaging_product": "whatsapp",
        "entity_name": r.entity_name,
        "entity_type": r.entity_type,
        "is_registered": r.is_registered,
    });
    if let Some(v) = r.other_entity_type {
        body["other_entity_type"] = json!(v);
    }
    if let Some(v) = r.grievance_officer_details {
        body["grievance_officer_details"] = v;
    }
    if let Some(v) = r.customer_care_details {
        body["customer_care_details"] = v;
    }
    let path = format!("{}/business_compliance_info", c.phone());
    c.call(Method::POST, &path, &[], Some(body)).await
}

fn analytics_fields(q: &AnalyticsQuery) -> Result<String, ApiError> {
    one_of(
        &q.granularity,
        &["HALF_HOUR", "DAY", "MONTH"],
        "granularity",
    )?;
    let mut f = format!(
        "analytics.start({}).end({}).granularity({})",
        q.start, q.end, q.granularity
    );
    if let Some(v) = &q.phone_numbers {
        f.push_str(&format!(
            ".phone_numbers({})",
            field_list(v, "phone_numbers")?
        ));
    }
    if let Some(v) = &q.country_codes {
        f.push_str(&format!(
            ".country_codes({})",
            field_list(v, "country_codes")?
        ));
    }
    Ok(f)
}

fn conversation_analytics_fields(q: &ConversationAnalyticsQuery) -> Result<String, ApiError> {
    one_of(
        &q.granularity,
        &["HALF_HOUR", "DAILY", "MONTHLY"],
        "granularity",
    )?;
    let mut f = format!(
        "conversation_analytics.start({}).end({}).granularity({})",
        q.start, q.end, q.granularity
    );
    let lists = [
        ("conversation_directions", &q.conversation_directions),
        ("dimensions", &q.dimensions),
        ("conversation_categories", &q.conversation_categories),
        ("conversation_types", &q.conversation_types),
        ("phone_numbers", &q.phone_numbers),
        ("country_codes", &q.country_codes),
    ];
    for (name, value) in lists {
        if let Some(v) = value {
            f.push_str(&format!(".{name}({})", field_list(v, name)?));
        }
    }
    Ok(f)
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/analytics",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID"), AnalyticsQuery),
    responses(
        (status = 200, description = "Sent/delivered message counts per data point"),
        (status = 400, description = "Invalid granularity or list values, or rejected by Meta")
    )
)]
pub async fn analytics(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<AnalyticsQuery>,
) -> Result<Json<Value>, ApiError> {
    let fields = analytics_fields(&q)?;
    let c = ctx(&state, &session_id).await?;
    c.call(Method::GET, &c.waba()?, &[("fields", fields)], None)
        .await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/conversation-analytics",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID"), ConversationAnalyticsQuery),
    responses(
        (status = 200, description = "Conversation counts and cost per data point"),
        (status = 400, description = "Invalid granularity or list values, or rejected by Meta")
    )
)]
pub async fn conversation_analytics(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<ConversationAnalyticsQuery>,
) -> Result<Json<Value>, ApiError> {
    let fields = conversation_analytics_fields(&q)?;
    let c = ctx(&state, &session_id).await?;
    c.call(Method::GET, &c.waba()?, &[("fields", fields)], None)
        .await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/credit-lines",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Lines of credit (extended credits) on the session's business"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn credit_lines(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/extendedcredits", c.business()?);
    let query = [("fields", "id,legal_entity_name".to_string())];
    c.call(Method::GET, &path, &query, None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/credit-lines/{credit_line_id}/allocations",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("credit_line_id" = String, Path, description = "Credit line ID")
    ),
    responses(
        (status = 200, description = "Credit sharing records on the credit line"),
        (status = 400, description = "Invalid credit_line_id, or rejected by Meta")
    )
)]
pub async fn credit_line_allocations(
    State(state): State<AppState>,
    Path((session_id, credit_line_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let id = path_id(&credit_line_id, "credit_line_id")?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{id}/owning_credit_allocation_configs");
    c.call(Method::GET, &path, &[], None).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/credit-sharing",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = ShareCreditLineRequest,
    responses(
        (status = 200, description = "Credit line shared with the session's WABA (allocation_config_id, waba_id)"),
        (status = 400, description = "Invalid credit_line_id, or rejected by Meta")
    )
)]
pub async fn share_credit_line(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<ShareCreditLineRequest>,
) -> Result<Json<Value>, ApiError> {
    let id = path_id(&r.credit_line_id, "credit_line_id")?;
    let c = ctx(&state, &session_id).await?;
    let query = [("waba_id", c.waba()?), ("waba_currency", r.waba_currency)];
    let path = format!("{id}/whatsapp_credit_sharing_and_attach");
    c.call(Method::POST, &path, &query, None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/credit-sharing/{allocation_config_id}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("allocation_config_id" = String, Path, description = "Credit sharing record ID")
    ),
    responses(
        (status = 200, description = "The credit sharing record, incl. its receiving credential"),
        (status = 400, description = "Invalid ID, or rejected by Meta")
    )
)]
pub async fn get_credit_sharing(
    State(state): State<AppState>,
    Path((session_id, allocation_config_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let id = path_id(&allocation_config_id, "allocation_config_id")?;
    let c = ctx(&state, &session_id).await?;
    let query = [(
        "fields",
        "id,receiving_credential{id},request_status,credential_type".to_string(),
    )];
    c.call(Method::GET, &id, &query, None).await
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/credit-sharing/{allocation_config_id}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("allocation_config_id" = String, Path, description = "Credit sharing record ID")
    ),
    responses(
        (status = 200, description = "Credit sharing revoked"),
        (status = 400, description = "Invalid ID, or rejected by Meta")
    )
)]
pub async fn revoke_credit_sharing(
    State(state): State<AppState>,
    Path((session_id, allocation_config_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let id = path_id(&allocation_config_id, "allocation_config_id")?;
    let c = ctx(&state, &session_id).await?;
    c.call(Method::DELETE, &id, &[], None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/assigned-users",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Users assigned to the session's WABA, with their tasks"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn list_assigned_users(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/assigned_users", c.waba()?);
    c.call(Method::GET, &path, &[("business", c.business()?)], None)
        .await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/assigned-users",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = AssignUserRequest,
    responses(
        (status = 200, description = "System user assigned to the WABA"),
        (status = 400, description = "Invalid user_id or tasks, or rejected by Meta")
    )
)]
pub async fn assign_user(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<AssignUserRequest>,
) -> Result<Json<Value>, ApiError> {
    let user = path_id(&r.user_id, "user_id")?;
    if r.tasks.is_empty() {
        return Err(ApiError::BadRequest("tasks must not be empty".to_string()));
    }
    let tasks = field_list(&r.tasks.join(","), "tasks")?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/assigned_users", c.waba()?);
    c.call(
        Method::POST,
        &path,
        &[("user", user), ("tasks", tasks)],
        None,
    )
    .await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/system-users",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "System users on the session's business"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn list_system_users(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/system_users", c.business()?);
    c.call(Method::GET, &path, &[], None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}/metrics",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID"),
        FlowMetricsQuery
    ),
    responses(
        (status = 200, description = "The requested Flow endpoint metric"),
        (status = 400, description = "Invalid metric, granularity or date, or rejected by Meta")
    )
)]
pub async fn flow_metrics(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
    Query(q): Query<FlowMetricsQuery>,
) -> Result<Json<Value>, ApiError> {
    let id = path_id(&flow_id, "flow_id")?;
    one_of(
        &q.metric,
        &[
            "ENDPOINT_REQUEST_COUNT",
            "ENDPOINT_REQUEST_ERROR",
            "ENDPOINT_REQUEST_ERROR_RATE",
            "ENDPOINT_REQUEST_LATENCY_SECONDS_CEIL",
            "ENDPOINT_AVAILABILITY",
        ],
        "metric",
    )?;
    one_of(&q.granularity, &["DAY", "HOUR", "LIFETIME"], "granularity")?;
    let mut field = format!("metric.name({}).granularity({})", q.metric, q.granularity);
    for (name, value) in [("since", &q.since), ("until", &q.until)] {
        if let Some(d) = value {
            if !is_date(d) {
                return Err(ApiError::BadRequest(format!("{name} must be YYYY-MM-DD")));
            }
            field.push_str(&format!(".{name}({d})"));
        }
    }
    let c = ctx(&state, &session_id).await?;
    c.call(Method::GET, &id, &[("fields", field)], None).await
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flow-endpoint/public-key",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "The Flows encryption public key Meta has on file for the number, and its signature status"),
        (status = 400, description = "Not a whatsapp_cloud session, invalid input, or rejected by Meta")
    )
)]
pub async fn flow_public_key(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/whatsapp_business_encryption", c.phone());
    c.call(Method::GET, &path, &[], None).await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows-migrate",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = MigrateFlowsRequest,
    responses(
        (status = 200, description = "Per-Flow migration results"),
        (status = 400, description = "Invalid source_waba_id, or rejected by Meta")
    )
)]
pub async fn migrate_flows(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<MigrateFlowsRequest>,
) -> Result<Json<Value>, ApiError> {
    let source = path_id(&r.source_waba_id, "source_waba_id")?;
    let c = ctx(&state, &session_id).await?;
    let mut form = reqwest::multipart::Form::new().text("source_waba_id", source);
    if let Some(names) = r.source_flow_names {
        form = form.text(
            "source_flow_names",
            serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string()),
        );
    }
    let path = format!("{}/migrate_flows", c.waba()?);
    c.client
        .graph_form(&path, form)
        .await
        .map(Json)
        .map_err(meta)
}

fn order_details_payload(r: &SendOrderDetailsRequest) -> Result<Value, ApiError> {
    let region = r.region.as_deref().unwrap_or("IN");
    one_of(region, &["IN", "SG"], "region")?;
    if !r.parameters.is_object() {
        return Err(ApiError::BadRequest(
            "parameters must be an object".to_string(),
        ));
    }
    let mut content = json!({ "body": { "text": r.body } });
    if let Some(header) = &r.header {
        content["header"] = header.clone();
    }
    if let Some(footer) = &r.footer {
        content["footer"] = json!({ "text": footer });
    }
    let interactive = if region == "SG" {
        content["action"] = r.parameters.clone();
        json!({ "type": "order_details", "order_details": content })
    } else {
        content["type"] = json!("order_details");
        content["action"] = json!({ "name": "review_and_pay", "parameters": r.parameters });
        content
    };
    Ok(json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": r.to,
        "type": "interactive",
        "interactive": interactive,
    }))
}

fn order_status_payload(r: &SendOrderStatusRequest) -> Value {
    let mut order = json!({ "status": r.status });
    if let Some(d) = &r.description {
        order["description"] = json!(d);
    }
    json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": r.to,
        "type": "interactive",
        "interactive": {
            "type": "order_status",
            "body": { "text": r.body },
            "action": {
                "name": "review_order",
                "parameters": { "reference_id": r.reference_id, "order": order },
            },
        },
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/order-details",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = SendOrderDetailsRequest,
    responses(
        (status = 200, description = "Order details (payment request) message sent", body = MessageResponse),
        (status = 400, description = "Invalid region/parameters, or rejected by Meta")
    )
)]
pub async fn send_order_details(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<SendOrderDetailsRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let payload = order_details_payload(&r)?;
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/messages", c.phone());
    let Json(resp) = c.call(Method::POST, &path, &[], Some(payload)).await?;
    Ok(sent(&resp, r.to))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/order-status",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = SendOrderStatusRequest,
    responses(
        (status = 200, description = "Order status update sent", body = MessageResponse),
        (status = 400, description = "Rejected by Meta")
    )
)]
pub async fn send_order_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(r): Json<SendOrderStatusRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let c = ctx(&state, &session_id).await?;
    let path = format!("{}/messages", c.phone());
    let Json(resp) = c
        .call(Method::POST, &path, &[], Some(order_status_payload(&r)))
        .await?;
    Ok(sent(&resp, r.to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_lists_become_json_arrays_and_reject_expansion_breakouts() {
        assert_eq!(field_list("US, BR", "c").unwrap(), r#"["US","BR"]"#);
        assert_eq!(field_list("+6281,", "c").unwrap(), r#"["+6281"]"#);
        for bad in ["US).x", "a\"b", "a.b", "a(b"] {
            assert!(field_list(bad, "c").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn analytics_fields_match_metas_expansion_syntax() {
        let q = AnalyticsQuery {
            start: 1680503760,
            end: 1680564980,
            granularity: "DAY".to_string(),
            phone_numbers: None,
            country_codes: Some("US,BR".to_string()),
        };
        assert_eq!(
            analytics_fields(&q).unwrap(),
            r#"analytics.start(1680503760).end(1680564980).granularity(DAY).country_codes(["US","BR"])"#
        );
        let bad = AnalyticsQuery {
            granularity: "WEEK".to_string(),
            ..q
        };
        assert!(analytics_fields(&bad).is_err());
    }

    #[test]
    fn conversation_analytics_fields_match_metas_expansion_syntax() {
        let q = ConversationAnalyticsQuery {
            start: 1656661480,
            end: 1674859480,
            granularity: "MONTHLY".to_string(),
            conversation_directions: Some("business_initiated".to_string()),
            dimensions: Some("conversation_type,conversation_direction".to_string()),
            conversation_categories: None,
            conversation_types: None,
            phone_numbers: None,
            country_codes: None,
        };
        assert_eq!(
            conversation_analytics_fields(&q).unwrap(),
            r#"conversation_analytics.start(1656661480).end(1674859480).granularity(MONTHLY).conversation_directions(["business_initiated"]).dimensions(["conversation_type","conversation_direction"])"#
        );
    }

    fn order(region: Option<&str>) -> SendOrderDetailsRequest {
        SendOrderDetailsRequest {
            to: "6281".to_string(),
            region: region.map(str::to_string),
            header: None,
            body: "Your order".to_string(),
            footer: Some("Thanks".to_string()),
            parameters: json!({"reference_id": "ref-1", "currency": "INR"}),
        }
    }

    #[test]
    fn order_details_nests_like_metas_india_example_by_default() {
        let p = order_details_payload(&order(None)).unwrap();
        let i = &p["interactive"];
        assert_eq!(i["type"], "order_details");
        assert_eq!(i["body"]["text"], "Your order");
        assert_eq!(i["footer"]["text"], "Thanks");
        assert_eq!(i["action"]["name"], "review_and_pay");
        assert_eq!(i["action"]["parameters"]["reference_id"], "ref-1");
        assert!(i.get("order_details").is_none());
    }

    #[test]
    fn order_details_nests_like_metas_singapore_example() {
        let p = order_details_payload(&order(Some("SG"))).unwrap();
        let i = &p["interactive"];
        assert_eq!(i["type"], "order_details");
        assert_eq!(i["order_details"]["body"]["text"], "Your order");
        assert_eq!(i["order_details"]["action"]["reference_id"], "ref-1");
        assert!(i.get("action").is_none());
        assert!(order_details_payload(&order(Some("US"))).is_err());
    }

    #[test]
    fn order_status_matches_metas_shape() {
        let p = order_status_payload(&SendOrderStatusRequest {
            to: "6281".to_string(),
            body: "Shipped".to_string(),
            reference_id: "ref-1".to_string(),
            status: "shipped".to_string(),
            description: None,
        });
        let a = &p["interactive"]["action"];
        assert_eq!(p["interactive"]["type"], "order_status");
        assert_eq!(a["name"], "review_order");
        assert_eq!(a["parameters"]["reference_id"], "ref-1");
        assert_eq!(a["parameters"]["order"], json!({"status": "shipped"}));
    }

    #[test]
    fn profile_update_needs_a_field_and_caps_websites() {
        let empty = UpdateBusinessProfileRequest {
            about: None,
            address: None,
            description: None,
            email: None,
            vertical: None,
            websites: None,
            profile_picture_handle: None,
        };
        assert!(profile_body(empty).is_err());
        let three = UpdateBusinessProfileRequest {
            about: None,
            address: None,
            description: None,
            email: None,
            vertical: None,
            websites: Some(vec!["a".into(), "b".into(), "c".into()]),
            profile_picture_handle: None,
        };
        assert!(profile_body(three).is_err());
        let ok = UpdateBusinessProfileRequest {
            about: Some("Open daily".into()),
            address: None,
            description: None,
            email: None,
            vertical: Some("RETAIL".into()),
            websites: None,
            profile_picture_handle: None,
        };
        let b = profile_body(ok).unwrap();
        assert_eq!(b["messaging_product"], "whatsapp");
        assert_eq!(b["about"], "Open daily");
        assert!(b.get("address").is_none());
    }

    #[test]
    fn block_body_matches_metas_shape() {
        assert!(block_body(vec![]).is_err());
        assert_eq!(
            block_body(vec!["6281".into()]).unwrap(),
            json!({"messaging_product": "whatsapp", "block_users": [{"user": "6281"}]})
        );
    }

    #[test]
    fn pins_must_be_six_digits() {
        assert!(check_pin("123456").is_ok());
        for bad in ["12345", "1234567", "12a456", ""] {
            assert!(check_pin(bad).is_err(), "{bad:?}");
        }
    }
}
