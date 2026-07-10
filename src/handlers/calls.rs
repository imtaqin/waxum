use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::Arc;
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::calls::{
    AcceptCallRequest, RejectCallRequest, RingCallRequest, RingCallResponse, TerminateCallRequest,
};
use crate::models::common::SuccessResponse;
use crate::state::{ActiveCallAudio, AppState};

fn make_dummy_audio() -> (
    async_channel::Receiver<Vec<i16>>,
    async_channel::Sender<Vec<i16>>,
    async_channel::Sender<Vec<i16>>,
    async_channel::Receiver<Vec<i16>>,
) {
    let (mic_tx, mic_rx) = async_channel::unbounded::<Vec<i16>>();
    let (spk_tx, spk_rx) = async_channel::unbounded::<Vec<i16>>();
    (mic_rx, spk_tx, mic_tx, spk_rx)
}

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
    if request.call_id.is_empty() {
        return Err(ApiError::BadRequest("call_id is empty".to_string()));
    }
    let incoming = state
        .incoming_calls()
        .remove(&request.call_id)
        .map(|(_, v)| v)
        .ok_or_else(|| ApiError::BadRequest("call_id not found in registry".to_string()))?;

    client
        .voip()
        .reject(&incoming)
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

    let (mic_rx, spk_tx, mic_tx, spk_rx) = make_dummy_audio();

    let handle = client
        .voip()
        .call(&to)
        .audio(mic_rx, spk_tx)
        .start()
        .await
        .map_err(|e| ApiError::Internal(format!("place_call failed: {e}")))?;

    let call_id = handle.call_id().to_string();
    let handle_arc = Arc::new(handle);
    state
        .active_calls()
        .insert(call_id.clone(), handle_arc);
    state
        .call_audio_channels()
        .insert(call_id.clone(), ActiveCallAudio { mic_tx, spk_rx });

    Ok(Json(RingCallResponse {
        call_id,
        to: to.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/accept",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = AcceptCallRequest,
    responses(
        (status = 200, description = "Accept signal sent", body = SuccessResponse),
        (status = 400, description = "Invalid caller JID or empty call_id"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn accept_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<AcceptCallRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    if request.call_id.is_empty() {
        return Err(ApiError::BadRequest("call_id is empty".to_string()));
    }
    let client = get_client(&state, &session_id)?;
    let incoming = state
        .incoming_calls()
        .remove(&request.call_id)
        .map(|(_, v)| v)
        .ok_or_else(|| ApiError::BadRequest("call_id not found in registry".to_string()))?;

    let (mic_rx, spk_tx, mic_tx, spk_rx) = make_dummy_audio();

    let handle = client
        .voip()
        .accept(&incoming)
        .audio(mic_rx, spk_tx)
        .start()
        .await
        .map_err(|e| ApiError::Internal(format!("accept failed: {e}")))?;

    let call_id = handle.call_id().to_string();
    state.active_calls().insert(call_id.clone(), Arc::new(handle));
    state
        .call_audio_channels()
        .insert(call_id, ActiveCallAudio { mic_tx, spk_rx });

    Ok(Json(SuccessResponse::with_message("Call accepted")))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/terminate",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = TerminateCallRequest,
    responses(
        (status = 200, description = "Terminate signal sent", body = SuccessResponse),
        (status = 400, description = "Invalid peer JID or empty call_id"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn terminate_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<TerminateCallRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    if request.call_id.is_empty() {
        return Err(ApiError::BadRequest("call_id is empty".to_string()));
    }
    let client = get_client(&state, &session_id)?;

    if let Some((_, handle)) = state.active_calls().remove(&request.call_id) {
        handle.hangup().await;
        state.call_audio_channels().remove(&request.call_id);
        return Ok(Json(SuccessResponse::with_message("Call terminated")));
    }

    let peer: Jid = request
        .peer
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.peer.clone()))?;

    client
        .voip()
        .terminate(&request.call_id, &peer, &peer)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Call terminated")))
}

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_live_client().ok_or(ApiError::NotConnected)
}
