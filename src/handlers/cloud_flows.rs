//! WhatsApp Flows for `whatsapp_cloud` sessions: Flow management (the
//! ordinary Graph API CRUD under `/cloud/flows`), sending a Flow as an
//! interactive message, and the Flows Data Exchange endpoint Meta calls
//! while a user is inside a Flow.
//!
//! The data-exchange route (`/cloud/flow-endpoint/exchange`) is exempt
//! from bearer auth in [`crate::middleware::jwt`], like `/cloud/webhook`,
//! because Meta calls it directly. Two things stand in for a waxum token
//! there: the `X-Hub-Signature-256` HMAC over the raw body (keyed by the
//! session's `cloud_app_secret`, checked first), and the RSA/AES-GCM
//! envelope itself (only the holder of the session's private key can
//! decrypt the request or produce a response Meta will accept -- see
//! [`crate::cloud::flows_crypto`]).
//!
//! Status codes on that route follow Meta's Flow endpoint contract:
//! `432` for a bad signature, `421` for any decryption failure (Meta
//! reacts by re-fetching the business public key), `200` with a base64
//! `text/plain` body otherwise. Every decryption failure maps to the same
//! `421` and the same body regardless of which step failed, so the route
//! doesn't tell a caller whether its RSA-wrapped key or its GCM payload
//! was the part that didn't check out.
//!
//! Non-ping requests (`INIT`, `data_exchange`, `BACK`, error
//! notifications) are decrypted and POSTed as JSON to the session's
//! configured `forward_url` through [`crate::net_guard::safe_http_client`];
//! its JSON reply is encrypted and handed back to Meta unchanged. With no
//! forward URL only the health-check ping is answered.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::cloud::client::CloudClient;
use crate::cloud::{flows_crypto, webhook};
use crate::db::session::CloudCredentials;
use crate::error::ApiError;
use crate::models::cloud::{
    ConfigureFlowEndpointRequest, ConfigureFlowEndpointResponse, CreateFlowRequest,
    SendFlowRequest, UpdateFlowMetadataRequest,
};
use crate::models::messages::MessageResponse;
use crate::state::AppState;

/// Meta's status for a request whose `X-Hub-Signature-256` doesn't match.
const STATUS_BAD_SIGNATURE: u16 = 432;
/// Meta's status for a request the endpoint couldn't decrypt; Meta
/// responds by re-downloading the business public key and retrying.
const STATUS_DECRYPT_FAILED: u16 = 421;

async fn cloud_creds(state: &AppState, session_id: &str) -> Result<CloudCredentials, ApiError> {
    state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("session is not a whatsapp_cloud session".to_string()))
}

fn client_for(creds: &CloudCredentials) -> CloudClient {
    CloudClient::new(&creds.phone_number_id, &creds.access_token)
}

fn upstream(e: crate::cloud::client::CloudError) -> ApiError {
    ApiError::Internal(format!("cloud flows call failed: {e}"))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = CreateFlowRequest,
    responses(
        (status = 200, description = "Flow created in draft status (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn create_flow(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<CreateFlowRequest>,
) -> Result<Json<Value>, ApiError> {
    if request.categories.is_empty() {
        return Err(ApiError::BadRequest(
            "categories must contain at least one category".to_string(),
        ));
    }
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .create_flow(
            &creds.waba_id,
            &request.name,
            &request.categories,
            request.clone_flow_id.as_deref(),
        )
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Flows on the session's WABA (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn list_flows(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .list_flows(&creds.waba_id)
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 200, description = "Flow details, validation errors and preview (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn get_flow(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .get_flow(&flow_id)
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID")
    ),
    request_body = UpdateFlowMetadataRequest,
    responses(
        (status = 200, description = "Flow metadata updated (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session, or nothing to update")
    )
)]
pub async fn update_flow_metadata(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
    Json(request): Json<UpdateFlowMetadataRequest>,
) -> Result<Json<Value>, ApiError> {
    if request.name.is_none() && request.categories.is_none() && request.endpoint_uri.is_none() {
        return Err(ApiError::BadRequest(
            "at least one of name, categories, endpoint_uri is required".to_string(),
        ));
    }
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .update_flow_metadata(
            &flow_id,
            request.name.as_deref(),
            request.categories.as_deref(),
            request.endpoint_uri.as_deref(),
        )
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 200, description = "Draft Flow deleted (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn delete_flow(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .delete_flow(&flow_id)
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}/json",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID")
    ),
    request_body(content = Object, description = "The Flow JSON document itself"),
    responses(
        (status = 200, description = "Flow JSON uploaded; response lists any validation errors"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn update_flow_json(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
    Json(flow_json): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let creds = cloud_creds(&state, &session_id).await?;
    let bytes = serde_json::to_vec(&flow_json)
        .map_err(|e| ApiError::BadRequest(format!("invalid flow JSON: {e}")))?;
    let resp = client_for(&creds)
        .update_flow_json(&flow_id, bytes)
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}/assets",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 200, description = "Flow assets incl. the Flow JSON download URL"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn get_flow_assets(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .get_flow_assets(&flow_id)
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}/publish",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 200, description = "Flow published (irreversible; Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn publish_flow(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .publish_flow(&flow_id)
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}/deprecate",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 200, description = "Published Flow deprecated (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn deprecate_flow(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .deprecate_flow(&flow_id)
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

/// Builds the `interactive.type = "flow"` message body, per "Send Flow".
fn flow_message_payload(flow_id: &str, request: &SendFlowRequest) -> Result<Value, ApiError> {
    let flow_action = request.flow_action.as_deref().unwrap_or("navigate");
    let mode = request.mode.as_deref().unwrap_or("published");
    if !matches!(flow_action, "navigate" | "data_exchange") {
        return Err(ApiError::BadRequest(
            "flow_action must be navigate or data_exchange".to_string(),
        ));
    }
    if !matches!(mode, "published" | "draft") {
        return Err(ApiError::BadRequest(
            "mode must be published or draft".to_string(),
        ));
    }

    let mut parameters = json!({
        "flow_message_version": "3",
        "flow_id": flow_id,
        "flow_cta": request.flow_cta,
        "flow_action": flow_action,
        "mode": mode,
    });
    if let Some(token) = &request.flow_token {
        parameters["flow_token"] = json!(token);
    }
    if flow_action == "navigate" {
        let screen = request.screen.as_deref().ok_or_else(|| {
            ApiError::BadRequest("screen is required when flow_action is navigate".to_string())
        })?;
        let mut action_payload = json!({ "screen": screen });
        if let Some(data) = &request.data {
            action_payload["data"] = data.clone();
        }
        parameters["flow_action_payload"] = action_payload;
    }

    let mut interactive = json!({
        "type": "flow",
        "body": { "text": request.body },
        "action": { "name": "flow", "parameters": parameters },
    });
    if let Some(header) = &request.header {
        interactive["header"] = json!({ "type": "text", "text": header });
    }
    if let Some(footer) = &request.footer {
        interactive["footer"] = json!({ "text": footer });
    }

    Ok(json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": request.to,
        "type": "interactive",
        "interactive": interactive,
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flows/{flow_id}/send",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("flow_id" = String, Path, description = "Flow ID")
    ),
    request_body = SendFlowRequest,
    responses(
        (status = 200, description = "Flow message sent", body = MessageResponse),
        (status = 400, description = "Not a whatsapp_cloud session, or invalid Flow parameters")
    )
)]
pub async fn send_flow(
    State(state): State<AppState>,
    Path((session_id, flow_id)): Path<(String, String)>,
    Json(request): Json<SendFlowRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let payload = flow_message_payload(&flow_id, &request)?;
    let creds = cloud_creds(&state, &session_id).await?;
    let resp = client_for(&creds)
        .send_raw(payload)
        .await
        .map_err(upstream)?;
    let message_id = resp
        .pointer("/messages/0/id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: request.to,
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/flow-endpoint",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = ConfigureFlowEndpointRequest,
    responses(
        (status = 200, description = "Public key registered with Meta, private key stored", body = ConfigureFlowEndpointResponse),
        (status = 400, description = "Not a whatsapp_cloud session, bad key, or non-public forward URL")
    )
)]
pub async fn configure_flow_endpoint(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<ConfigureFlowEndpointRequest>,
) -> Result<Json<ConfigureFlowEndpointResponse>, ApiError> {
    let creds = cloud_creds(&state, &session_id).await?;

    if let Some(url) = &request.forward_url {
        crate::net_guard::validate_public_url(url)
            .await
            .map_err(ApiError::BadRequest)?;
    }

    let private_key = match &request.private_key {
        Some(pem) => flows_crypto::parse_private_key(pem)
            .map_err(|_| ApiError::BadRequest("private_key is not a valid RSA PEM".to_string()))?,
        None => tokio::task::spawn_blocking(flows_crypto::generate_private_key)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .map_err(|e| ApiError::Internal(format!("RSA key generation failed: {e}")))?,
    };
    if private_key.key_size_bits() < 2048 {
        return Err(ApiError::BadRequest(
            "private_key must be at least 2048 bits".to_string(),
        ));
    }
    let private_pem = flows_crypto::private_key_pem(&private_key)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let public_pem = flows_crypto::public_key_pem(&private_key)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let meta_response = client_for(&creds)
        .set_flow_encryption_public_key(&public_pem)
        .await
        .map_err(upstream)?;

    state
        .session_manager()
        .set_flow_endpoint(&session_id, &private_pem, request.forward_url.as_deref())
        .await?;

    Ok(Json(ConfigureFlowEndpointResponse {
        public_key: public_pem,
        endpoint_path: format!("/api/v1/sessions/{session_id}/cloud/flow-endpoint/exchange"),
        meta_response,
    }))
}

fn plain_status(code: u16, body: &'static str) -> Response {
    (
        StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_REQUEST),
        body,
    )
        .into_response()
}

/// `POST /sessions/{session_id}/cloud/flow-endpoint/exchange` -- the
/// Flows Data Exchange endpoint Meta calls directly. See the module docs
/// for the auth model and status-code contract.
pub async fn flow_data_exchange(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(Some(creds)) = state
        .session_manager()
        .get_cloud_credentials(&session_id)
        .await
    else {
        return plain_status(404, "unknown cloud session");
    };
    let Some(private_key_pem) = creds.flow_private_key.as_deref() else {
        return plain_status(404, "flow endpoint not configured");
    };

    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok());
    if !webhook::verify_signature(&creds.app_secret, &body, signature) {
        tracing::warn!(session_id = %session_id, "flow endpoint: signature verification failed");
        return plain_status(STATUS_BAD_SIGNATURE, "invalid signature");
    }

    let Ok(envelope) = serde_json::from_slice::<Value>(&body) else {
        return plain_status(400, "invalid JSON");
    };

    let decrypted = match flows_crypto::decrypt_request(private_key_pem, &envelope) {
        Ok(d) => d,
        Err(flows_crypto::FlowCryptoError::Malformed(_)) => {
            return plain_status(400, "malformed request")
        }
        Err(e) => {
            tracing::warn!(session_id = %session_id, error = %e, "flow endpoint: decryption failed");
            return plain_status(STATUS_DECRYPT_FAILED, "decryption failed");
        }
    };

    let reply = if flows_crypto::is_ping(&decrypted.body) {
        flows_crypto::ping_response()
    } else {
        let Some(forward_url) = creds.flow_forward_url.as_deref() else {
            tracing::warn!(session_id = %session_id, "flow endpoint: no forward_url configured for a non-ping request");
            return plain_status(503, "no flow handler configured");
        };
        match forward(forward_url, &session_id, &decrypted.body).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "flow endpoint: forward failed");
                return plain_status(502, "flow handler failed");
            }
        }
    };

    match flows_crypto::encrypt_response(&decrypted, &reply) {
        Ok(ciphertext) => {
            (StatusCode::OK, [("content-type", "text/plain")], ciphertext).into_response()
        }
        Err(_) => plain_status(500, "encryption failed"),
    }
}

/// POSTs a decrypted Flow request to the business's handler and returns
/// its JSON reply. Uses the SSRF-safe client, so a forward URL that
/// re-resolves to a private address after configuration is still refused.
async fn forward(url: &str, session_id: &str, body: &Value) -> Result<Value, String> {
    let resp = crate::net_guard::safe_http_client()
        .post(url)
        .header("X-Waxum-Session-Id", session_id)
        .json(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("handler returned {}", resp.status()));
    }
    resp.json::<Value>().await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> SendFlowRequest {
        SendFlowRequest {
            to: "6281234567890".to_string(),
            flow_cta: "Book".to_string(),
            body: "Pick a slot".to_string(),
            header: Some("Clinic".to_string()),
            footer: None,
            flow_token: Some("tok-1".to_string()),
            mode: None,
            flow_action: None,
            screen: Some("WELCOME".to_string()),
            data: Some(json!({"name": "Ada"})),
        }
    }

    #[test]
    fn navigate_payload_matches_metas_send_flow_shape() {
        let p = flow_message_payload("123", &request()).unwrap();
        assert_eq!(p["type"], "interactive");
        assert_eq!(p["interactive"]["type"], "flow");
        assert_eq!(p["interactive"]["header"]["text"], "Clinic");
        assert!(p["interactive"].get("footer").is_none());
        let params = &p["interactive"]["action"]["parameters"];
        assert_eq!(p["interactive"]["action"]["name"], "flow");
        assert_eq!(params["flow_message_version"], "3");
        assert_eq!(params["flow_id"], "123");
        assert_eq!(params["flow_action"], "navigate");
        assert_eq!(params["mode"], "published");
        assert_eq!(params["flow_token"], "tok-1");
        assert_eq!(params["flow_action_payload"]["screen"], "WELCOME");
        assert_eq!(params["flow_action_payload"]["data"]["name"], "Ada");
    }

    #[test]
    fn data_exchange_payload_has_no_action_payload() {
        let mut r = request();
        r.flow_action = Some("data_exchange".to_string());
        r.screen = None;
        let p = flow_message_payload("123", &r).unwrap();
        assert!(p["interactive"]["action"]["parameters"]
            .get("flow_action_payload")
            .is_none());
    }

    #[test]
    fn navigate_without_screen_is_rejected() {
        let mut r = request();
        r.screen = None;
        assert!(flow_message_payload("123", &r).is_err());
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let mut r = request();
        r.mode = Some("live".to_string());
        assert!(flow_message_payload("123", &r).is_err());
    }
}
