//! Regression probe for the FTS5 search query.
//!
//! Guards against the ambiguous-column bug where the search SELECT
//! joined `messages_fts` and `messages` (both expose
//! message_id/session_id/body) with UNQUALIFIED column names, so the
//! FTS query errored at prepare time and every search silently fell
//! back to the snippet-less LIKE path. The columns must be qualified
//! with `m.`; this test replicates the exact production query and
//! asserts a real `<b>…</b>` snippet comes back.

use waxum::db::sqlite_raw::{self, Value as SQ};

const COLS: &str =
    "m.id, m.message_id, m.session_id, m.chat_jid, m.sender_jid, m.direction, m.msg_type, m.body, m.msg_timestamp";

#[test]
fn fts5_query_returns_snippet() {
    let dir = std::env::temp_dir().join(format!("ftsprobe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("probe.db");
    let handle = sqlite_raw::open(db.to_str().unwrap()).expect("open");
    let conn = handle.lock();

    sqlite_raw::exec_batch(
        &conn,
        "CREATE TABLE messages ( \
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            message_id TEXT NOT NULL, session_id TEXT NOT NULL, chat_jid TEXT NOT NULL, \
            sender_jid TEXT NOT NULL, direction TEXT NOT NULL, msg_type TEXT NOT NULL, \
            body TEXT, msg_timestamp TEXT NOT NULL, \
            UNIQUE (session_id, message_id) ); \
         CREATE VIRTUAL TABLE messages_fts USING fts5(body, session_id UNINDEXED, message_id UNINDEXED);",
    )
    .expect("setup");

    sqlite_raw::execute(
        &conn,
        "INSERT INTO messages (message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp) VALUES (?,?,?,?,?,?,?,?)",
        &[
            SQ::Text("m1".into()), SQ::Text("s1".into()), SQ::Text("c@w".into()),
            SQ::Text("".into()), SQ::Text("out".into()), SQ::Text("text".into()),
            SQ::Text("lunch at noon works for me".into()), SQ::Text("2026-07-23 10:00:00".into()),
        ],
    ).expect("insert msg");
    sqlite_raw::execute(
        &conn,
        "INSERT INTO messages_fts (body, session_id, message_id) VALUES (?,?,?)",
        &[
            SQ::Text("lunch at noon works for me".into()),
            SQ::Text("s1".into()),
            SQ::Text("m1".into()),
        ],
    )
    .expect("insert fts");

    let sql = format!(
        "SELECT {COLS}, snippet(messages_fts, 0, '<b>', '</b>', '…', 16) FROM messages_fts f JOIN messages m ON m.session_id = f.session_id AND m.message_id = f.message_id WHERE messages_fts MATCH ? ORDER BY m.msg_timestamp DESC, m.id DESC LIMIT ? OFFSET ?"
    );
    let rows = sqlite_raw::query(
        &conn,
        &sql,
        &[SQ::Text("\"lunch\"".into()), SQ::Int(20), SQ::Int(0)],
        |r| r.get_string(9),
    )
    .expect("FTS query should prepare and run without ambiguity errors");

    assert_eq!(rows.len(), 1, "expected one hit");
    let snippet = rows[0].clone().unwrap_or_default();
    assert!(
        snippet.contains("<b>lunch</b>"),
        "snippet should be highlighted, was: {snippet:?}"
    );
}
