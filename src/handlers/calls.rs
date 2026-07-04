use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::builder::NodeBuilder;
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::calls::{RejectCallRequest, RingCallRequest, RingCallResponse};
use crate::models::common::SuccessResponse;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/reject",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RejectCallRequest,
    responses(
        (status = 200, description = "Call rejected", body = SuccessResponse),
        (status = 400, description = "Invalid JID or empty call_id"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn reject_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RejectCallRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let from: Jid = request
        .from
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.from.clone()))?;

    client
        .reject_call(&request.call_id, &from)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Call rejected")))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/ring",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RingCallRequest,
    responses(
        (status = 200, description = "Ring signal sent", body = RingCallResponse),
        (status = 400, description = "Invalid recipient"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn ring_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RingCallRequest>,
) -> Result<Json<RingCallResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let to: Jid = if request.to.contains('@') {
        request
            .to
            .parse()
            .map_err(|_| ApiError::InvalidJid(request.to.clone()))?
    } else {
        Jid::pn(&request.to)
    };

    let call_creator = client
        .get_pn()
        .ok_or_else(|| ApiError::Internal("session has no phone JID yet".to_string()))?;

    let call_id = request
        .call_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string().to_uppercase());
    let stanza_id = uuid::Uuid::new_v4().simple().to_string().to_uppercase();

    let audio_16k = NodeBuilder::new("audio")
        .attr("enc", "opus")
        .attr("rate", "16000")
        .build();
    let audio_8k = NodeBuilder::new("audio")
        .attr("enc", "opus")
        .attr("rate", "8000")
        .build();

    let offer = NodeBuilder::new("offer")
        .attr("call-id", call_id.as_str())
        .attr("call-creator", &call_creator)
        .children([audio_16k, audio_8k])
        .build();

    let stanza = NodeBuilder::new("call")
        .attr("to", &to)
        .attr("id", stanza_id.as_str())
        .children([offer])
        .build();

    client
        .send_node(stanza)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(RingCallResponse {
        call_id,
        to: to.to_string(),
    }))
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
