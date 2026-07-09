use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::builder::NodeBuilder;
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::calls::{
    AcceptCallRequest, RejectCallRequest, RingCallRequest, RingCallResponse, TerminateCallRequest,
};
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

    if request.call_id.is_empty() {
        return Err(ApiError::BadRequest("call_id is empty".to_string()));
    }

    let id = uuid::Uuid::new_v4().simple().to_string().to_uppercase();
    let stanza = NodeBuilder::new("call")
        .attr("to", &from)
        .attr("id", id.as_str())
        .children([NodeBuilder::new("reject")
            .attr("call-id", request.call_id.as_str())
            .attr("call-creator", &from)
            .attr("count", "0")
            .build()])
        .build();

    client
        .send_node(stanza)
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
    let push_name = client.get_push_name();
    let notify = if push_name.is_empty() {
        "wa-rs".to_string()
    } else {
        push_name
    };

    let call_id = request
        .call_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string().to_uppercase());
    let stanza_id = uuid::Uuid::new_v4().simple().to_string().to_uppercase();
    let now_ts = chrono::Utc::now().timestamp().to_string();

    let is_video = request
        .kind
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
        == Some("video");

    let audio_16k = NodeBuilder::new("audio")
        .attr("enc", "opus")
        .attr("rate", "16000")
        .build();
    let audio_8k = NodeBuilder::new("audio")
        .attr("enc", "opus")
        .attr("rate", "8000")
        .build();

    let mut offer_children = vec![audio_16k, audio_8k];
    if is_video {
        offer_children.push(
            NodeBuilder::new("video")
                .attr("enc", "vp8")
                .attr("orientation", "0")
                .attr("screen_width", "1280")
                .attr("screen_height", "720")
                .build(),
        );
    }

    let offer = NodeBuilder::new("offer")
        .attr("call-id", call_id.as_str())
        .attr("call-creator", &call_creator)
        .attr("caller_pn", &call_creator)
        .attr("device_class", "2016")
        .attr("joinable", "1")
        .children(offer_children)
        .build();

    let stanza = NodeBuilder::new("call")
        .attr("to", &to)
        .attr("id", stanza_id.as_str())
        .attr("from", &call_creator)
        .attr("version", "2.25.37.76")
        .attr("platform", "android")
        .attr("notify", notify.as_str())
        .attr("t", now_ts.as_str())
        .attr("e", "0")
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
    let from: Jid = request
        .from
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.from.clone()))?;

    let accept = NodeBuilder::new("accept")
        .attr("call-id", request.call_id.as_str())
        .attr("call-creator", &from)
        .build();

    let stanza = NodeBuilder::new("call")
        .attr("to", &from)
        .attr(
            "id",
            uuid::Uuid::new_v4().simple().to_string().to_uppercase(),
        )
        .children([accept])
        .build();

    client
        .send_node(stanza)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

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
    let peer: Jid = request
        .peer
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.peer.clone()))?;
    let reason = request.reason.as_deref().unwrap_or("hangup");

    let terminate = NodeBuilder::new("terminate")
        .attr("call-id", request.call_id.as_str())
        .attr("call-creator", &peer)
        .attr("reason", reason)
        .build();

    let stanza = NodeBuilder::new("call")
        .attr("to", &peer)
        .attr(
            "id",
            uuid::Uuid::new_v4().simple().to_string().to_uppercase(),
        )
        .children([terminate])
        .build();

    client
        .send_node(stanza)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Call terminated")))
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
