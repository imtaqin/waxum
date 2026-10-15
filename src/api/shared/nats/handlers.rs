use axum::{extract::State, Json};
use serde_json::json;
use utoipa::OpenApi;

use crate::error::ApiError;
use crate::nats::models::{NatsStatusResponse, NatsStreamInfo};
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    paths(nats_status, nats_purge_stream, nats_list_consumers),
    components(schemas(NatsStatusResponse, NatsStreamInfo))
)]
#[allow(dead_code)]
pub struct NatsApi;

/// Get NATS JetStream status
///
/// Returns connection status and stream information.
#[utoipa::path(
    get,
    path = "/api/v1/nats/status",
    tag = "nats",
    responses(
        (status = 200, description = "NATS status", body = NatsStatusResponse),
    )
)]
pub async fn nats_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let nats = state.nats();

    if nats.is_none() {
        return Json(json!(NatsStatusResponse {
            enabled: false,
            connected: false,
            url: None,
            events_stream: None,
            send_stream: None,
        }));
    }

    let nats = nats.unwrap();
    let js = nats.jetstream();
    let config = nats.config();

    let events_info = get_stream_info(js, &config.events_stream).await;
    let send_info = get_stream_info(js, &config.send_stream).await;

    Json(json!(NatsStatusResponse {
        enabled: true,
        connected: true,
        url: Some(config.url.clone()),
        events_stream: events_info,
        send_stream: send_info,
    }))
}

/// Purge a NATS JetStream stream
///
/// Removes all messages from the specified stream.
#[utoipa::path(
    post,
    path = "/api/v1/nats/streams/{stream_name}/purge",
    tag = "nats",
    params(
        ("stream_name" = String, Path, description = "Stream name to purge (e.g., WA_EVENTS or WA_SEND)")
    ),
    responses(
        (status = 200, description = "Stream purged successfully"),
        (status = 500, description = "NATS error"),
    )
)]
pub async fn nats_purge_stream(
    State(state): State<AppState>,
    axum::extract::Path(stream_name): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let nats = state
        .nats()
        .ok_or_else(|| ApiError::NatsError("NATS is not enabled".into()))?;

    let stream = nats
        .jetstream()
        .get_stream(&stream_name)
        .await
        .map_err(|e| ApiError::NatsError(format!("Stream not found: {}", e)))?;

    stream
        .purge()
        .await
        .map_err(|e| ApiError::NatsError(format!("Failed to purge: {}", e)))?;

    Ok(Json(json!({
        "success": true,
        "message": format!("Stream '{}' purged", stream_name)
    })))
}

/// List NATS JetStream consumers
///
/// Returns all consumers for the specified stream.
#[utoipa::path(
    get,
    path = "/api/v1/nats/streams/{stream_name}/consumers",
    tag = "nats",
    params(
        ("stream_name" = String, Path, description = "Stream name (e.g., WA_EVENTS or WA_SEND)")
    ),
    responses(
        (status = 200, description = "List of consumers"),
        (status = 500, description = "NATS error"),
    )
)]
pub async fn nats_list_consumers(
    State(state): State<AppState>,
    axum::extract::Path(stream_name): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let nats = state
        .nats()
        .ok_or_else(|| ApiError::NatsError("NATS is not enabled".into()))?;

    let mut stream = nats
        .jetstream()
        .get_stream(&stream_name)
        .await
        .map_err(|e| ApiError::NatsError(format!("Stream not found: {}", e)))?;

    let info = stream
        .info()
        .await
        .map_err(|e| ApiError::NatsError(format!("Failed to get stream info: {}", e)))?;

    Ok(Json(json!({
        "success": true,
        "stream": stream_name,
        "consumer_count": info.state.consumer_count,
    })))
}

async fn get_stream_info(
    js: &async_nats::jetstream::Context,
    stream_name: &str,
) -> Option<NatsStreamInfo> {
    let mut stream = js.get_stream(stream_name).await.ok()?;
    let info = stream.info().await.ok()?;

    Some(NatsStreamInfo {
        name: stream_name.to_string(),
        messages: info.state.messages,
        bytes: info.state.bytes,
        consumer_count: info.state.consumer_count,
        first_seq: info.state.first_sequence,
        last_seq: info.state.last_sequence,
    })
}
