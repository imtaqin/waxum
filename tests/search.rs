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

use waxum::db::contacts::{ContactStore, ContactUpsert};
use waxum::db::messages::{insert, MediaPointer, NewMessage};

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
        media: None,
        quoted_message_id: None,
        quoted_sender_jid: None,
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

/// `search()`'s SQLite FTS5 path reads its row positionally
/// (`sqlite_row_to_message`), from a SELECT built from a separate
/// column-list constant (`m_cols`) than `list_by_chat`'s -- a
/// regression here would silently shift every field after
/// `quoted_message_id`/`quoted_sender_jid` (snippet included) rather
/// than fail to compile, so this exercises the real HTTP endpoint
/// through the FTS5 path specifically (a handful of rows, same as
/// `search_finds_seeded_rows_with_snippet`) and checks the quoted
/// fields land correctly alongside every other column.
#[tokio::test]
async fn search_surfaces_quoted_context_through_fts5_path() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-quoted").await;

    let mut reply = msg(
        "MID-REPLY",
        "search-s-quoted",
        "in",
        "text",
        Some("sure, lunch works"),
        ts(2),
    );
    reply.quoted_message_id = Some("MID-ORIGINAL".to_string());
    reply.quoted_sender_jid = Some("559999999999@s.whatsapp.net".to_string());
    insert(&h.pool, &reply).await.expect("insert reply");

    insert(
        &h.pool,
        &msg(
            "MID-PLAIN",
            "search-s-quoted",
            "in",
            "text",
            Some("lunch plans for later"),
            ts(1),
        ),
    )
    .await
    .expect("insert plain");

    let (status, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-s-quoted/messages/search?q=lunch",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let hits = body["messages"].as_array().expect("messages array");
    assert_eq!(hits.len(), 2);

    let reply_hit = hits
        .iter()
        .find(|h| h["message_id"] == "MID-REPLY")
        .expect("reply hit present");
    assert_eq!(reply_hit["quoted_message_id"], "MID-ORIGINAL");
    assert_eq!(
        reply_hit["quoted_sender_jid"],
        "559999999999@s.whatsapp.net"
    );
    assert_eq!(reply_hit["chat_jid"], "559999999999@s.whatsapp.net");
    assert_eq!(reply_hit["body"], "sure, lunch works");

    let plain_hit = hits
        .iter()
        .find(|h| h["message_id"] == "MID-PLAIN")
        .expect("plain hit present");
    assert_eq!(plain_hit["quoted_message_id"], serde_json::Value::Null);
    assert_eq!(plain_hit["quoted_sender_jid"], serde_json::Value::Null);
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

#[tokio::test]
async fn chat_listing_includes_push_name_and_media_pointer() {
    let h = Harness::new().await;
    seed_session(&h, "search-s-08").await;

    ContactStore::new(&h.pool)
        .upsert(&ContactUpsert {
            session_id: "search-s-08",
            jid: "559999999999@s.whatsapp.net",
            push_name: Some("Jane Doe"),
            source: "message",
            ..Default::default()
        })
        .await
        .expect("contact upsert");

    let mut text_row = msg(
        "MID-C1",
        "search-s-08",
        "in",
        "text",
        Some("hey there"),
        ts(2),
    );
    text_row.chat_jid = "120000000000000000@g.us".to_string();
    insert(&h.pool, &text_row).await.expect("insert");

    let mut image_row = msg("MID-C2", "search-s-08", "in", "image", None, ts(1));
    image_row.chat_jid = "120000000000000000@g.us".to_string();
    image_row.media = Some(MediaPointer {
        media_key: "a2V5".to_string(),
        file_sha256: "c2hh".to_string(),
        file_enc_sha256: "ZW5j".to_string(),
        direct_path: "/v/t/abc".to_string(),
        file_length: 1234,
        media_type: "image".to_string(),
        mimetype: "image/jpeg".to_string(),
    });
    image_row.quoted_message_id = Some("MID-C1".to_string());
    image_row.quoted_sender_jid = Some("559999999999@s.whatsapp.net".to_string());
    insert(&h.pool, &image_row).await.expect("insert");

    let (status, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/search-s-08/messages/chat/120000000000000000@g.us",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 2);
    let hits = body["messages"].as_array().expect("messages array");

    assert_eq!(hits[0]["message_id"], "MID-C2");
    assert_eq!(hits[0]["push_name"], "Jane Doe");
    assert_eq!(hits[0]["media"]["media_key"], "a2V5");
    assert_eq!(hits[0]["media"]["file_sha256"], "c2hh");
    assert_eq!(hits[0]["media"]["direct_path"], "/v/t/abc");
    assert_eq!(hits[0]["media"]["file_length"], 1234);
    assert_eq!(hits[0]["media"]["media_type"], "image");
    assert_eq!(hits[0]["quoted_message_id"], "MID-C1");
    assert_eq!(hits[0]["quoted_sender_jid"], "559999999999@s.whatsapp.net");

    assert_eq!(hits[1]["message_id"], "MID-C1");
    assert_eq!(hits[1]["push_name"], "Jane Doe");
    assert_eq!(hits[1]["media"], serde_json::Value::Null);
    assert_eq!(hits[1]["quoted_message_id"], serde_json::Value::Null);
    assert_eq!(hits[1]["quoted_sender_jid"], serde_json::Value::Null);
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
