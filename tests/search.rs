//! Tests for message history search.
//!
//! Rows are seeded directly through [`waxum::db::messages::insert`]
//! into the harness pool (no live WhatsApp client needed), then the
//! search endpoints are exercised through the full HTTP pipeline on a
//! temp SQLite DB — including the FTS5 path with `snippet()`
//! highlights, since the bundled SQLite build ships FTS5.
mod common;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

use waxum::db::messages::{insert, NewMessage};

async fn seed_session(h: &Harness, id: &str) {
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": id, "name": id}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

fn msg(
    message_id: &str,
    session_id: &str,
    direction: &str,
    msg_type: &str,
    body: Option<&str>,
    ts: chrono::DateTime<Utc>,
) -> NewMessage {
    NewMessage {
        message_id: message_id.to_string(),
        session_id: session_id.to_string(),
        chat_jid: "559999999999@s.whatsapp.net".to_string(),
        sender_jid: if direction == "in" {
            "559999999999@s.whatsapp.net".to_string()
        } else {
            String::new()
        },
        direction: direction.to_string(),
        msg_type: msg_type.to_string(),
        body: body.map(str::to_string),
        msg_timestamp: ts,
    }
}

fn ts(hours_ago: i64) -> chrono::DateTime<Utc> {
    Utc::now() - Duration::hours(hours_ago)
}

#[tokio::test]
async fn search_finds_seeded_rows_with_snippet() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-01").await;

    insert(
        &h.pool,
        &msg(
            "MID-1",
            "search-s-01",
            "in",
            "text",
            Some("are we still on for lunch tomorrow?"),
            ts(3),
        ),
    )
    .await
    .expect("insert");
    insert(
        &h.pool,
        &msg(
            "MID-2",
            "search-s-01",
            "out",
            "text",
            Some("lunch at noon works for me"),
            ts(2),
        ),
    )
    .await
    .expect("insert");
    insert(
        &h.pool,
        &msg(
            "MID-3",
            "search-s-01",
            "in",
            "text",
            Some("dinner is booked"),
            ts(1),
        ),
    )
    .await
    .expect("insert");

    let (status, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-s-01/messages/search?q=lunch",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 2);
    let hits = body["messages"].as_array().expect("messages array");
    assert_eq!(hits[0]["message_id"], "MID-2");
    assert_eq!(hits[0]["direction"], "out");
    assert_eq!(hits[1]["message_id"], "MID-1");
    assert_eq!(hits[1]["direction"], "in");
    let snippet = hits[0]["snippet"].as_str().unwrap_or_default();
    assert!(snippet.contains("<b>lunch</b>"), "snippet was: {snippet}");
    assert_eq!(hits[0]["chat_jid"], "559999999999@s.whatsapp.net");
    assert_eq!(hits[0]["msg_type"], "text");
}

#[tokio::test]
async fn duplicate_message_id_is_stored_once() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-02").await;

    let row = msg(
        "MID-DUP",
        "search-s-02",
        "in",
        "text",
        Some("echo echo echo"),
        ts(1),
    );
    insert(&h.pool, &row).await.expect("insert");
    insert(&h.pool, &row).await.expect("insert again");

    let (_, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-s-02/messages/search?q=echo",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn search_paginates_with_limit_and_offset() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-03").await;

    for i in 0..5i64 {
        insert(
            &h.pool,
            &msg(
                &format!("MID-P{i}"),
                "search-s-03",
                "in",
                "text",
                Some("invoice reminder for this month"),
                Utc::now() - Duration::minutes(i),
            ),
        )
        .await
        .expect("insert");
    }

    let (_, page1) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-s-03/messages/search?q=invoice&limit=2&offset=0",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(page1["count"], 2);
    assert_eq!(page1["messages"][0]["message_id"], "MID-P0");

    let (_, page2) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-s-03/messages/search?q=invoice&limit=2&offset=2",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(page2["count"], 2);
    assert_eq!(page2["messages"][0]["message_id"], "MID-P2");

    let p1_ids: Vec<&str> = page1["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["message_id"].as_str())
        .collect();
    let p2_ids: Vec<&str> = page2["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["message_id"].as_str())
        .collect();
    for id in &p2_ids {
        assert!(!p1_ids.contains(id));
    }
}

#[tokio::test]
async fn fleet_search_scopes_and_filters_sessions() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-04a").await;
    seed_session(&h, "search-s-04b").await;

    insert(
        &h.pool,
        &msg(
            "MID-F1",
            "search-s-04a",
            "in",
            "text",
            Some("shared keyword alpha"),
            ts(2),
        ),
    )
    .await
    .expect("insert");
    insert(
        &h.pool,
        &msg(
            "MID-F2",
            "search-s-04b",
            "out",
            "text",
            Some("shared keyword beta"),
            ts(1),
        ),
    )
    .await
    .expect("insert");

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/messages/search?q=keyword", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 2);
    assert_eq!(body["messages"][0]["message_id"], "MID-F2");
    assert_eq!(body["messages"][0]["session_id"], "search-s-04b");

    let (_, body) = call(
        &h.app,
        req_get(
            "/api/v1/messages/search?q=keyword&session=search-s-04a",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(body["count"], 1);
    assert_eq!(body["messages"][0]["message_id"], "MID-F1");
}

#[tokio::test]
async fn search_validation_auth_and_404() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-05").await;

    let (status, _) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-s-05/messages/search?q=%20%20",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/messages/search?q=", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-missing/messages/search?q=lunch",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/search-s-05/messages/search?q=lunch", None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn no_match_returns_empty_page() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-06").await;

    insert(
        &h.pool,
        &msg("MID-Z", "search-s-06", "in", "text", Some("hello"), ts(1)),
    )
    .await
    .expect("insert");

    let (status, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-s-06/messages/search?q=zzzznothing",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 0);
    assert_eq!(body["messages"], json!([]));
}

#[tokio::test]
async fn special_match_syntax_in_query_does_not_error() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-07").await;

    insert(
        &h.pool,
        &msg(
            "MID-S",
            "search-s-07",
            "in",
            "text",
            Some("price is 100% final"),
            ts(1),
        ),
    )
    .await
    .expect("insert");

    for q in ["100%\"", "OR AND NEAR", "100%"] {
        let (status, _) = call(
            &h.app,
            req_get(
                &format!(
                    "/api/v1/sessions/search-s-07/messages/search?q={}",
                    urlencoding(q)
                ),
                Some(TEST_TOKEN),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "query {q} should not error");
    }
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
