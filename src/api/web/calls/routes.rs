//! Routes for calls.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/voices", get(handlers::list_voices))
        .route("/tts/preview", get(handlers::tts_preview))
}

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/calls/reject", post(handlers::reject_call))
        .route("/{session_id}/calls/ring", post(handlers::ring_call))
        .route("/{session_id}/calls/tts", post(handlers::tts_call))
        .route("/{session_id}/calls/play", post(handlers::play_call))
        .route(
            "/{session_id}/calls/media/ws",
            get(handlers::media_stream_ws),
        )
        .route(
            "/{session_id}/calls/{call_id}/recording.wav",
            get(handlers::get_recording),
        )
        .route(
            "/{session_id}/calls/{call_id}/transcript",
            post(handlers::transcribe_call),
        )
        .route("/{session_id}/calls/accept", post(handlers::accept_call))
        .route(
            "/{session_id}/calls/terminate",
            post(handlers::terminate_call),
        )
        .route(
            "/{session_id}/calls/video-orientation",
            post(handlers::set_call_video_orientation),
        )
}
