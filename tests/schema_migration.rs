use std::collections::HashSet;

use waxum::db::{
    schema,
    session::DbPool,
    sqlite_raw::{self, Value},
};

#[tokio::test]
async fn sqlite_upgrade_adds_message_media_columns() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("waxum.db");
    let handle = sqlite_raw::open(db_path.to_str().expect("utf-8 path")).expect("open sqlite");

    {
        let conn = handle.lock();
        sqlite_raw::exec_batch(
            &conn,
            "CREATE TABLE messages (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                message_id TEXT NOT NULL, \
                session_id TEXT NOT NULL, \
                chat_jid TEXT NOT NULL, \
                sender_jid TEXT NOT NULL, \
                direction TEXT NOT NULL, \
                msg_type TEXT NOT NULL, \
                body TEXT, \
                msg_timestamp TEXT NOT NULL, \
                created_at TEXT NOT NULL, \
                UNIQUE (session_id, message_id)\
            );",
        )
        .expect("create legacy messages table");
    }

    let pool = DbPool::SQLite(handle.clone());
    schema::init_schema(&pool).await.expect("migrate schema");
    schema::init_schema(&pool).await.expect("repeat migration");

    let conn = handle.lock();
    let columns: HashSet<String> =
        sqlite_raw::query(&conn, "PRAGMA table_info(messages)", &[], |row| {
            row.get_string(1).expect("column name")
        })
        .expect("read columns")
        .into_iter()
        .collect();

    for expected in [
        "media_key",
        "file_sha256",
        "file_enc_sha256",
        "direct_path",
        "file_length",
        "media_type",
        "mimetype",
    ] {
        assert!(
            columns.contains(expected),
            "missing migrated column {expected}"
        );
    }

    sqlite_raw::execute(
        &conn,
        "INSERT INTO messages (message_id, session_id, chat_jid, sender_jid, direction, \
         msg_type, body, msg_timestamp, created_at, media_key, file_sha256, file_enc_sha256, \
         direct_path, file_length, media_type, mimetype) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::Text("msg-1".into()),
            Value::Text("session-1".into()),
            Value::Text("chat@s.whatsapp.net".into()),
            Value::Text("sender@s.whatsapp.net".into()),
            Value::Text("in".into()),
            Value::Text("image".into()),
            Value::Text("caption".into()),
            Value::Text("2026-08-17 00:00:00".into()),
            Value::Text("2026-08-17 00:00:00".into()),
            Value::Text("key".into()),
            Value::Text("sha".into()),
            Value::Text("enc-sha".into()),
            Value::Text("/media/path".into()),
            Value::Int(42),
            Value::Text("image".into()),
            Value::Text("image/jpeg".into()),
        ],
    )
    .expect("insert message with migrated media columns");
}

#[tokio::test]
async fn sqlite_upgrade_adds_quoted_message_columns() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("waxum.db");
    let handle = sqlite_raw::open(db_path.to_str().expect("utf-8 path")).expect("open sqlite");

    {
        let conn = handle.lock();
        sqlite_raw::exec_batch(
            &conn,
            "CREATE TABLE messages (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                message_id TEXT NOT NULL, \
                session_id TEXT NOT NULL, \
                chat_jid TEXT NOT NULL, \
                sender_jid TEXT NOT NULL, \
                direction TEXT NOT NULL, \
                msg_type TEXT NOT NULL, \
                body TEXT, \
                msg_timestamp TEXT NOT NULL, \
                created_at TEXT NOT NULL, \
                UNIQUE (session_id, message_id)\
            );",
        )
        .expect("create legacy messages table");
    }

    let pool = DbPool::SQLite(handle.clone());
    schema::init_schema(&pool).await.expect("migrate schema");
    schema::init_schema(&pool).await.expect("repeat migration");

    let conn = handle.lock();
    let columns: HashSet<String> =
        sqlite_raw::query(&conn, "PRAGMA table_info(messages)", &[], |row| {
            row.get_string(1).expect("column name")
        })
        .expect("read columns")
        .into_iter()
        .collect();

    for expected in ["quoted_message_id", "quoted_sender_jid"] {
        assert!(
            columns.contains(expected),
            "missing migrated column {expected}"
        );
    }

    sqlite_raw::execute(
        &conn,
        "INSERT INTO messages (message_id, session_id, chat_jid, sender_jid, direction, \
         msg_type, body, msg_timestamp, created_at, quoted_message_id, quoted_sender_jid) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::Text("msg-2".into()),
            Value::Text("session-1".into()),
            Value::Text("chat@s.whatsapp.net".into()),
            Value::Text("sender@s.whatsapp.net".into()),
            Value::Text("in".into()),
            Value::Text("text".into()),
            Value::Text("reply body".into()),
            Value::Text("2026-08-17 00:00:00".into()),
            Value::Text("2026-08-17 00:00:00".into()),
            Value::Text("quoted-msg-id".into()),
            Value::Text("quoter@s.whatsapp.net".into()),
        ],
    )
    .expect("insert message with migrated quote columns");
}
