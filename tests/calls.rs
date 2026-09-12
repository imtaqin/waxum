//! Contract tests for the call / TTS handlers (`src/handlers/calls.rs`).
//!
//! The base pattern is the one documented in `tests/presence.rs`: assert the
//! HTTP contract up to the `get_client` gate, never the protocol behaviour
//! past it. `calls` follows that pattern for its seven JSON `POST` routes,
//! but it is also the only module in the set with routes that are not plain
//! JSON request/response, so three deviations are pinned here deliberately.
//!
//! # Deviation 1 — validation order is not uniform
//!
//! Five of the seven `POST` handlers reject a trivially-invalid field
//! (`text`, `audio_url`, `call_id`) *before* calling `get_client`, so an
//! unknown session plus an empty field is `400`, not `503`. `reject_call`
//! is the exception: it resolves the client first, so the identical request
//! shape gets `503` and the `call_id is empty` branch is unreachable until a
//! session is live. Both orders are pinned below so the asymmetry is visible
//! rather than accidental.
//!
//! # Deviation 2 — two routes have no client gate at all
//!
//! `GET .../recording.wav` reads the recording store, and
//! `POST .../transcript` reads `WHISPER_API_URL` and the recording store.
//! Neither calls `get_client`, so neither can return `503`. A recording is
//! therefore served for a session that has never connected, which is the
//! intended behaviour: a recording outlives the call that produced it.
//!
//! # Deviation 3 — `media/ws` is an upgrade, not a request/response
//!
//! `WebSocketUpgrade` is a `FromRequestParts` extractor, and in
//! `media_stream_ws` it is declared after `Path` and `Query` and therefore
//! runs before the handler body. The consequence, asserted below, is that
//! the client gate is never reached over a plain request: a non-upgrade
//! `GET` is rejected at extraction with `400`, and a well-formed upgrade
//! request driven through `tower::ServiceExt::oneshot` gets `426`, because
//! `oneshot` calls the router directly and no hyper server has inserted the
//! `OnUpgrade` extension. That is the contract for this route under an
//! integration test; exercising the socket itself needs a bound listener and
//! a live WhatsApp client, which is out of scope here.
//!
//! # What the fleet routes can and cannot assert
//!
//! `GET /api/v1/voices` and `GET /api/v1/tts/preview` sit outside the session
//! scope and have no client gate. `list_voices` calls Edge-TTS on the first
//! request, so only its auth contract is asserted — anything further would
//! make the suite depend on a Microsoft endpoint. `tts_preview` validates
//! `text` before it opens that socket, so its input contract is assertable
//! offline and is covered; the 500-character upper bound is only checked from
//! above for the same reason.

mod common;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use common::{bearer, call, call_bytes, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::{json, Value};

const SESSION: &str = "s-calls";

/// Creates a session row (no live client is ever attached in tests).
async fn seed_session(h: &Harness, id: &str) {
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({ "id": id }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "seeding session {id} should succeed: {body:?}"
    );
}

/// Every session-scoped `POST` route with a body that is valid enough to
/// reach the handler body, so the only thing left to fail is the client gate.
fn session_post_routes(session_id: &str) -> Vec<(String, Value)> {
    vec![
        (
            format!("/api/v1/sessions/{session_id}/calls/reject"),
            json!({ "from": "628123456789@s.whatsapp.net", "call_id": "CALL1" }),
        ),
        (
            format!("/api/v1/sessions/{session_id}/calls/ring"),
            json!({ "to": "628123456789" }),
        ),
        (
            format!("/api/v1/sessions/{session_id}/calls/tts"),
            json!({ "to": "628123456789", "text": "halo" }),
        ),
        (
            format!("/api/v1/sessions/{session_id}/calls/play"),
            json!({ "to": "628123456789", "audio_url": "https://example.com/a.mp3" }),
        ),
        (
            format!("/api/v1/sessions/{session_id}/calls/accept"),
            json!({ "from": "628123456789@s.whatsapp.net", "call_id": "CALL1" }),
        ),
        (
            format!("/api/v1/sessions/{session_id}/calls/terminate"),
            json!({ "peer": "628123456789@s.whatsapp.net", "call_id": "CALL1" }),
        ),
        (
            format!("/api/v1/sessions/{session_id}/calls/video-orientation"),
            json!({ "call_id": "CALL1", "orientation": 0 }),
        ),
    ]
}

#[tokio::test]
async fn call_routes_require_a_token() {
    let h = Harness::new().await;

    for (path, body) in session_post_routes(SESSION) {
        let (status, _) = call(&h.app, req_json(Method::POST, &path, None, body.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "unauthenticated {path}");
    }

    let unauthenticated_gets = [
        format!("/api/v1/sessions/{SESSION}/calls/CALL1/recording.wav"),
        format!("/api/v1/sessions/{SESSION}/calls/media/ws?to=628123456789"),
        "/api/v1/voices".to_string(),
        "/api/v1/tts/preview?text=hi&voice=id-ID-ArdiNeural".to_string(),
    ];
    for path in unauthenticated_gets {
        let (status, _) = call(&h.app, req_get(&path, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "unauthenticated {path}");
    }

    let (transcript_status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/calls/CALL1/transcript"),
            None,
            json!({}),
        ),
    )
    .await;
    assert_eq!(transcript_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn call_routes_reject_an_unknown_session() {
    let h = Harness::new().await;

    for (path, body) in session_post_routes("does-not-exist") {
        let (status, _) = call(
            &h.app,
            req_json(Method::POST, &path, Some(TEST_TOKEN), body.clone()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} should fail at the client gate"
        );
    }
}

/// The same assertion as above, but with the session row present. `get_client`
/// reads the live registry rather than the `sessions` table, so creating the
/// session changes nothing — 503, never 404.
#[tokio::test]
async fn call_routes_reject_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    for (path, body) in session_post_routes(SESSION) {
        let (status, _) = call(
            &h.app,
            req_json(Method::POST, &path, Some(TEST_TOKEN), body.clone()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} has a DB row but no live client"
        );
    }
}

/// `Json<T>` is the last extractor on every one of these handlers, so it runs
/// before the handler body and a body that does not deserialize never reaches
/// the client gate.
#[tokio::test]
async fn call_routes_reject_a_malformed_body_before_the_client_gate() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    for (path, _) in session_post_routes(SESSION) {
        let (status, _) = call(
            &h.app,
            req_json(Method::POST, &path, Some(TEST_TOKEN), json!({})),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path}: extraction must fail before the client gate is reached"
        );
        assert!(
            status.is_client_error(),
            "{path}: a body missing every required field is a client error, got {status}"
        );
    }
}

/// `orientation` is a `u8`, so `4` deserializes fine and the 0-3 range check
/// lives past the client gate — but `256` overflows the type and is rejected
/// at extraction. Pinning this keeps the documented "orientation out of 0-3
/// range" 400 honest about which half of the range is enforced where.
#[tokio::test]
async fn video_orientation_rejects_a_value_that_overflows_u8() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/calls/video-orientation"),
            Some(TEST_TOKEN),
            json!({ "call_id": "CALL1", "orientation": 256 }),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "an out-of-range u8 is a client error, got {status}"
    );
    assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// Deviation 1, first half: `tts`, `play`, `accept`, `terminate` and
/// `video-orientation` check their trivially-invalid field before resolving
/// the client, so these are 400 even though the session does not exist.
#[tokio::test]
async fn most_call_routes_validate_empty_fields_before_the_client_gate() {
    let h = Harness::new().await;

    let cases = [
        (
            "/api/v1/sessions/does-not-exist/calls/tts",
            json!({ "to": "628123456789", "text": "   " }),
        ),
        (
            "/api/v1/sessions/does-not-exist/calls/play",
            json!({ "to": "628123456789", "audio_url": "  " }),
        ),
        (
            "/api/v1/sessions/does-not-exist/calls/accept",
            json!({ "from": "628123456789@s.whatsapp.net", "call_id": "" }),
        ),
        (
            "/api/v1/sessions/does-not-exist/calls/terminate",
            json!({ "peer": "628123456789@s.whatsapp.net", "call_id": "" }),
        ),
        (
            "/api/v1/sessions/does-not-exist/calls/video-orientation",
            json!({ "call_id": "", "orientation": 0 }),
        ),
    ];

    for (path, body) in cases {
        let (status, _) = call(&h.app, req_json(Method::POST, path, Some(TEST_TOKEN), body)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} validates the body before resolving the client"
        );
    }
}

/// Deviation 1, second half. `reject_call` calls `get_client` first, so the
/// same empty-`call_id` request that is 400 on the five handlers above is 503
/// here. This is an ordering asymmetry within one module, not a behaviour
/// anyone relies on — pinned so a future change to either order is a visible
/// test failure rather than a silent status-code change.
#[tokio::test]
async fn reject_call_resolves_the_client_before_validating_call_id() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/calls/reject"),
            Some(TEST_TOKEN),
            json!({ "from": "628123456789@s.whatsapp.net", "call_id": "" }),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "reject_call gates on the client first, so `call_id is empty` is unreachable here"
    );
}

/// Deviation 3. A plain `GET` carries none of the handshake headers, so
/// `WebSocketUpgrade` rejects at extraction — before the handler body, and
/// therefore before `get_client`. The session in this request does not exist,
/// so the absence of a 503 is what proves the ordering.
#[tokio::test]
async fn media_ws_rejects_a_request_that_is_not_an_upgrade() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/does-not-exist/calls/media/ws?to=628123456789",
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "the upgrade extractor must run before the client gate"
    );
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a GET without `Connection: upgrade` is rejected by WebSocketUpgrade, got {status}"
    );
}

/// `Query<MediaWsQuery>` is declared before `WebSocketUpgrade`, so a missing
/// `to` is rejected first and never reaches either the upgrade check or the
/// client gate.
#[tokio::test]
async fn media_ws_requires_the_to_query_parameter() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/calls/media/ws"),
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `to` is a client error, got {status}"
    );
    assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// A well-formed handshake gets past every header check in `WebSocketUpgrade`
/// and fails on the last one: `oneshot` drives the router directly, so no
/// hyper server has inserted the `OnUpgrade` extension and the connection is
/// not upgradable. This is the ceiling for `media/ws` in this harness, and the
/// reason the socket itself is not exercised here.
#[tokio::test]
async fn media_ws_upgrade_is_not_completable_through_oneshot() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/v1/sessions/{SESSION}/calls/media/ws?to=628123456789"
        ))
        .header(header::AUTHORIZATION, bearer(TEST_TOKEN))
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_VERSION, "13")
        .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .expect("build request");

    let (status, _) = call(&h.app, req).await;

    assert_eq!(
        status,
        StatusCode::UPGRADE_REQUIRED,
        "no OnUpgrade extension exists when the router is called directly, got {status}"
    );
}

/// Deviation 2. `get_recording` never calls `get_client`, so a missing
/// recording is 404 with a JSON error body — not the 503 every other
/// session-scoped route returns for a session that has no live client.
#[tokio::test]
async fn recording_returns_404_json_when_the_recording_is_absent() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, headers, body) = call_bytes(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/calls/CALL1/recording.wav"),
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json"),
        "the not-found branch is JSON even though the route serves audio"
    );
    let parsed: Value = serde_json::from_slice(&body).expect("404 body is JSON");
    assert_eq!(parsed["success"], json!(false));
    assert_eq!(parsed["error"]["code"], json!(404));
}

/// The success path: a recording written to the store is served verbatim as
/// `audio/wav`. The session has a database row but has never connected, which
/// pins the other half of deviation 2 — a recording outlives the call, so this
/// route deliberately does not require a live client.
#[tokio::test]
async fn recording_is_served_as_wav_for_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let wav = fake_wav();
    h.state
        .recordings()
        .write(SESSION, "CALL1", wav.clone())
        .await
        .expect("write recording");

    let (status, headers, body) = call_bytes(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/calls/CALL1/recording.wav"),
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("audio/wav")
    );
    assert_eq!(
        headers
            .get(header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok()),
        Some("attachment; filename=\"CALL1.wav\"")
    );
    assert_eq!(body, wav, "the stored bytes are served unmodified");
    assert!(body.starts_with(b"RIFF"));
}

/// Recordings are keyed by session, so one session cannot read another's.
#[tokio::test]
async fn recording_is_scoped_to_its_own_session() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;
    seed_session(&h, "s-calls-other").await;

    h.state
        .recordings()
        .write(SESSION, "CALL1", fake_wav())
        .await
        .expect("write recording");

    let (status, _, _) = call_bytes(
        &h.app,
        req_get(
            "/api/v1/sessions/s-calls-other/calls/CALL1/recording.wav",
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Deviation 2, the transcript half. `transcribe_call` has no `get_client`
/// call, so an unknown session cannot produce 503: it fails on
/// `WHISPER_API_URL` or on the missing recording, both of which are
/// `ApiError::Internal`.
#[tokio::test]
async fn transcript_never_reaches_the_client_gate() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/does-not-exist/calls/CALL1/transcript",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "transcribe_call has no client gate"
    );
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// `tts_preview` validates `text` before it opens a socket to Edge-TTS, so
/// these three cases are assertable without any network access. Anything past
/// them reaches Microsoft's endpoint and is not exercised here.
#[tokio::test]
async fn tts_preview_rejects_bad_input_before_reaching_edge_tts() {
    let h = Harness::new().await;

    let (empty_status, _) = call(
        &h.app,
        req_get(
            "/api/v1/tts/preview?text=&voice=id-ID-ArdiNeural",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(empty_status, StatusCode::BAD_REQUEST, "empty text");

    let long_text = "a".repeat(501);
    let (long_status, _) = call(
        &h.app,
        req_get(
            &format!("/api/v1/tts/preview?text={long_text}&voice=id-ID-ArdiNeural"),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(long_status, StatusCode::BAD_REQUEST, "text over 500 chars");

    let (missing_voice_status, _) = call(
        &h.app,
        req_get("/api/v1/tts/preview?text=hi", Some(TEST_TOKEN)),
    )
    .await;
    assert!(
        missing_voice_status.is_client_error(),
        "`voice` has no serde default, so Query extraction fails, got {missing_voice_status}"
    );
}

/// A minimal RIFF/WAV blob. The bytes only have to round-trip through the
/// recording store, so this is a header and nothing else.
fn fake_wav() -> Vec<u8> {
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&36u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&0u32.to_le_bytes());
    wav
}
