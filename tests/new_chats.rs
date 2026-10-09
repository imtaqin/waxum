//! New-outgoing-chat counter and limit (#157): the settings endpoint, and
//! what `admit` counts and refuses.
mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_delete, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;
use wacore_binary::jid::Jid;
use waxum::db::messages::{self, NewMessage};
use waxum::error::ApiError;
use waxum::handlers::new_chats::admit;

const LIMIT_PATH: &str = "/api/v1/sessions/nc-1/settings/new-chat-limit";

async fn session(h: &Harness) {
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "nc-1"}),
        ),
    )
    .await;
    assert!(status.is_success());
}

fn jid(s: &str) -> Jid {
    s.parse().expect("valid jid")
}

#[tokio::test]
async fn the_limit_is_off_by_default_validated_and_removable() {
    let h = Harness::new().await;
    session(&h).await;

    let (status, body) = call(&h.app, req_get(LIMIT_PATH, Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["limit"].is_null(),
        "no limit unless one is set: {body}"
    );
    assert_eq!(body["new_chats"]["last_24h"], 0);
    assert_eq!(body["incidents"], json!([]));

    for bad in [
        json!({"max_new_chats": 0}),
        json!({"max_new_chats": -3}),
        json!({"max_new_chats": 10, "window_hours": 0}),
        json!({"max_new_chats": 10, "window_hours": 100000}),
    ] {
        let (status, _) = call(
            &h.app,
            req_json(Method::PUT, LIMIT_PATH, Some(TEST_TOKEN), bad.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }

    let (status, body) = call(
        &h.app,
        req_json(
            Method::PUT,
            LIMIT_PATH,
            Some(TEST_TOKEN),
            json!({"max_new_chats": 40}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["limit"],
        json!({"max_new_chats": 40, "window_hours": 24})
    );

    let (status, body) = call(&h.app, req_delete(LIMIT_PATH, Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["limit"].is_null());

    let (status, _) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/nope/settings/new-chat-limit",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_new_direct_chats_count_and_only_they_are_refused() {
    let h = Harness::new().await;
    session(&h).await;
    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            LIMIT_PATH,
            Some(TEST_TOKEN),
            json!({"max_new_chats": 2, "window_hours": 24}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let a = "628111111111@s.whatsapp.net";
    let b = "628222222222@s.whatsapp.net";
    let c = "628333333333@s.whatsapp.net";
    let wrote_first = "628444444444@s.whatsapp.net";
    let group = "120363000000000001@g.us";

    messages::insert(
        &h.pool,
        &NewMessage {
            message_id: "IN-1".into(),
            session_id: "nc-1".into(),
            chat_jid: wrote_first.into(),
            sender_jid: wrote_first.into(),
            direction: "in".into(),
            msg_type: "text".into(),
            body: Some("hello".into()),
            msg_timestamp: chrono::Utc::now(),
            media: None,
            quoted_message_id: None,
            quoted_sender_jid: None,
        },
    )
    .await
    .unwrap();

    admit(&h.state, "nc-1", &jid(a), a)
        .await
        .expect("1st new chat");
    admit(&h.state, "nc-1", &jid(b), "628222222222")
        .await
        .expect("2nd new chat");
    admit(&h.state, "nc-1", &jid(a), a)
        .await
        .expect("an existing chat is never refused");
    admit(&h.state, "nc-1", &jid(wrote_first), wrote_first)
        .await
        .expect("replying to someone who wrote first is not a new chat");
    admit(&h.state, "nc-1", &jid(group), group)
        .await
        .expect("groups are not counted");

    match admit(&h.state, "nc-1", &jid(c), c).await {
        Err(ApiError::NewChatLimit {
            max_new_chats,
            window_hours,
            retry_after_secs,
        }) => {
            assert_eq!((max_new_chats, window_hours), (2, 24));
            assert!(retry_after_secs > 23 * 3600 && retry_after_secs <= 24 * 3600);
        }
        other => panic!("the 3rd new chat must hit the limit, got {other:?}"),
    }

    let (_, body) = call(&h.app, req_get(LIMIT_PATH, Some(TEST_TOKEN))).await;
    assert_eq!(body["new_chats"]["last_3h"], 2, "{body}");
    assert_eq!(body["new_chats"]["last_24h"], 2);
    assert!(body["retry_after_seconds"].as_i64().unwrap() > 0);

    let (_, status_body) = call(
        &h.app,
        req_get("/api/v1/sessions/nc-1/status", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(
        status_body["diagnostics"]["new_chats"]["last_24h"], 2,
        "{status_body}"
    );

    let (status, _) = call(&h.app, req_delete(LIMIT_PATH, Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    admit(&h.state, "nc-1", &jid(c), c)
        .await
        .expect("without a limit nothing is refused");
    let (_, body) = call(&h.app, req_get(LIMIT_PATH, Some(TEST_TOKEN))).await;
    assert_eq!(body["new_chats"]["last_24h"], 3, "counting continues");
}

/// A bulk send to many new numbers at once must not slip past the limit:
/// check and record are serialised per session.
#[tokio::test]
async fn concurrent_first_contacts_cannot_exceed_the_limit() {
    let h = Harness::new().await;
    session(&h).await;
    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            LIMIT_PATH,
            Some(TEST_TOKEN),
            json!({"max_new_chats": 5}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let mut sends = Vec::new();
    for n in 0..40 {
        let state = h.state.clone();
        sends.push(tokio::spawn(async move {
            let number = format!("6285{n:08}@s.whatsapp.net");
            admit(&state, "nc-1", &jid(&number), &number).await.is_ok()
        }));
    }
    let mut admitted = 0;
    for send in sends {
        if send.await.unwrap() {
            admitted += 1;
        }
    }
    assert_eq!(admitted, 5);
}

/// A send that fails after `admit` must not leave a counted chat behind.
#[tokio::test]
async fn a_failed_send_is_not_counted() {
    use waxum::handlers::new_chats::{forget, scope};

    let h = Harness::new().await;
    session(&h).await;
    let number = "628999999999@s.whatsapp.net";

    let (outcome, recorded) = scope(admit(&h.state, "nc-1", &jid(number), number)).await;
    outcome.expect("admitted");
    assert_eq!(recorded.len(), 1);
    let (_, body) = call(&h.app, req_get(LIMIT_PATH, Some(TEST_TOKEN))).await;
    assert_eq!(body["new_chats"]["last_24h"], 1);

    forget(recorded).await;
    let (_, body) = call(&h.app, req_get(LIMIT_PATH, Some(TEST_TOKEN))).await;
    assert_eq!(body["new_chats"]["last_24h"], 0, "rolled back: {body}");

    let (_, recorded) = scope(admit(&h.state, "nc-1", &jid(number), number)).await;
    assert_eq!(recorded.len(), 1, "and it counts as new again next time");
}
