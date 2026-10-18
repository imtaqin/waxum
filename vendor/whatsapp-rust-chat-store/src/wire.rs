//! JSON forms of the store's rows, for serving a chat store over HTTP and
//! reading it back on the other side (a remote client renders exactly what
//! a local store would give it). Messages carry their protobuf as base64.

use std::str::FromStr;

use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use wacore_binary::jid::Jid;

use crate::types::{
    ChatEntry, ContactEntry, MessageKind, MessageStatus, ReactionEntry, ReceiptEntry,
    StoreChange, StoredMessage,
};

fn ms(t: DateTime<Utc>) -> i64 {
    t.timestamp_millis()
}

fn time(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default()
}

fn jid(raw: &str) -> Jid {
    if raw.is_empty() {
        Jid::default()
    } else {
        Jid::from_str(raw).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireChat {
    pub jid: String,
    pub name: Option<String>,
    pub last_message_at_ms: Option<i64>,
    pub last_message_preview: Option<String>,
    pub last_message_kind: Option<String>,
    pub unread_count: i32,
    pub pinned_at_ms: Option<i64>,
    pub muted_until_ms: Option<i64>,
    pub archived: bool,
    pub ephemeral_expiration: Option<u32>,
}

impl From<&ChatEntry> for WireChat {
    fn from(c: &ChatEntry) -> Self {
        Self {
            jid: c.jid.to_string(),
            name: c.name.clone(),
            last_message_at_ms: c.last_message_at.map(ms),
            last_message_preview: c.last_message_preview.clone(),
            last_message_kind: c.last_message_kind.as_ref().map(|k| k.as_str().to_string()),
            unread_count: c.unread_count,
            pinned_at_ms: c.pinned_at.map(ms),
            muted_until_ms: c.muted_until.map(ms),
            archived: c.archived,
            ephemeral_expiration: c.ephemeral_expiration,
        }
    }
}

impl From<WireChat> for ChatEntry {
    fn from(c: WireChat) -> Self {
        Self {
            jid: jid(&c.jid),
            name: c.name,
            last_message_at: c.last_message_at_ms.map(time),
            last_message_preview: c.last_message_preview,
            last_message_kind: c.last_message_kind.map(MessageKind::from_db),
            unread_count: c.unread_count,
            pinned_at: c.pinned_at_ms.map(time),
            muted_until: c.muted_until_ms.map(time),
            archived: c.archived,
            ephemeral_expiration: c.ephemeral_expiration,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMessage {
    pub chat_jid: String,
    pub id: String,
    pub sender_jid: String,
    pub from_me: bool,
    pub timestamp_ms: i64,
    pub kind: String,
    pub text: Option<String>,
    /// The `waproto` Message, protobuf-encoded, base64.
    pub proto: Option<String>,
    pub status: i32,
    pub starred: bool,
    pub edited_at_ms: Option<i64>,
    pub revoked: bool,
    pub seq: i64,
}

impl From<&StoredMessage> for WireMessage {
    fn from(m: &StoredMessage) -> Self {
        let proto = m.message.as_ref().map(|msg| {
            let mut bytes = Vec::with_capacity(waproto::codec::message_encoded_len(msg));
            waproto::codec::message_encode_into(msg, &mut bytes);
            base64::engine::general_purpose::STANDARD.encode(bytes)
        });
        Self {
            chat_jid: m.chat_jid.to_string(),
            id: m.id.clone(),
            sender_jid: m.sender_jid.to_string(),
            from_me: m.from_me,
            timestamp_ms: ms(m.timestamp),
            kind: m.kind.as_str().to_string(),
            text: m.text.clone(),
            proto,
            status: m.status as i32,
            starred: m.starred,
            edited_at_ms: m.edited_at.map(ms),
            revoked: m.revoked,
            seq: m.seq,
        }
    }
}

impl From<WireMessage> for StoredMessage {
    fn from(m: WireMessage) -> Self {
        let message = m
            .proto
            .as_deref()
            .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
            .and_then(|bytes| waproto::codec::message_decode(&bytes).ok())
            .map(Box::new);
        Self {
            chat_jid: jid(&m.chat_jid),
            id: m.id,
            sender_jid: jid(&m.sender_jid),
            from_me: m.from_me,
            timestamp: time(m.timestamp_ms),
            kind: MessageKind::from_db(m.kind),
            text: m.text,
            message,
            status: MessageStatus::from_raw(m.status),
            starred: m.starred,
            edited_at: m.edited_at_ms.map(time),
            revoked: m.revoked,
            seq: m.seq,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireReaction {
    pub sender_jid: String,
    pub emoji: String,
    pub timestamp_ms: i64,
}

impl From<&ReactionEntry> for WireReaction {
    fn from(r: &ReactionEntry) -> Self {
        Self {
            sender_jid: r.sender_jid.to_string(),
            emoji: r.emoji.clone(),
            timestamp_ms: ms(r.timestamp),
        }
    }
}

impl From<WireReaction> for ReactionEntry {
    fn from(r: WireReaction) -> Self {
        Self {
            sender_jid: jid(&r.sender_jid),
            emoji: r.emoji,
            timestamp: time(r.timestamp_ms),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireReceipt {
    pub user_jid: String,
    pub status: i32,
    pub timestamp_ms: i64,
}

impl From<&ReceiptEntry> for WireReceipt {
    fn from(r: &ReceiptEntry) -> Self {
        Self {
            user_jid: r.user_jid.to_string(),
            status: r.status as i32,
            timestamp_ms: ms(r.timestamp),
        }
    }
}

impl From<WireReceipt> for ReceiptEntry {
    fn from(r: WireReceipt) -> Self {
        Self {
            user_jid: jid(&r.user_jid),
            status: MessageStatus::from_raw(r.status),
            timestamp: time(r.timestamp_ms),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireContact {
    pub jid: String,
    pub push_name: Option<String>,
    pub full_name: Option<String>,
    pub first_name: Option<String>,
    pub business_name: Option<String>,
}

impl From<&ContactEntry> for WireContact {
    fn from(c: &ContactEntry) -> Self {
        Self {
            jid: c.jid.to_string(),
            push_name: c.push_name.clone(),
            full_name: c.full_name.clone(),
            first_name: c.first_name.clone(),
            business_name: c.business_name.clone(),
        }
    }
}

impl From<WireContact> for ContactEntry {
    fn from(c: WireContact) -> Self {
        Self {
            jid: jid(&c.jid),
            push_name: c.push_name,
            full_name: c.full_name,
            first_name: c.first_name,
            business_name: c.business_name,
        }
    }
}

/// What a remote client is told as it happens: store changes plus the
/// realtime signals the store does not keep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    Chats,
    Messages {
        chat: String,
    },
    Contacts,
    Presence {
        jid: String,
        available: bool,
        last_seen_ms: Option<i64>,
    },
    Typing {
        chat: String,
        sender: String,
        group: bool,
        /// "composing" or "paused".
        state: String,
        audio: bool,
    },
    Picture {
        jid: String,
    },
}

impl From<&StoreChange> for WireEvent {
    fn from(change: &StoreChange) -> Self {
        match change {
            StoreChange::Chats => Self::Chats,
            StoreChange::Messages { chat } => Self::Messages {
                chat: chat.to_string(),
            },
            StoreChange::Contacts => Self::Contacts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_with_their_proto() {
        let mut msg = waproto::whatsapp::Message::default();
        msg.conversation = Some("halo".into());
        let stored = StoredMessage {
            chat_jid: jid("62811@s.whatsapp.net"),
            id: "ABC".into(),
            sender_jid: jid("62811@s.whatsapp.net"),
            from_me: false,
            timestamp: time(1_700_000_000_000),
            kind: MessageKind::Text,
            text: Some("halo".into()),
            message: Some(Box::new(msg)),
            status: MessageStatus::Read,
            starred: true,
            edited_at: None,
            revoked: false,
            seq: 42,
        };
        let json = serde_json::to_string(&WireMessage::from(&stored)).unwrap();
        let back: StoredMessage = serde_json::from_str::<WireMessage>(&json).unwrap().into();
        assert_eq!(back.id, "ABC");
        assert_eq!(back.chat_jid, stored.chat_jid);
        assert_eq!(back.timestamp, stored.timestamp);
        assert_eq!(back.kind, MessageKind::Text);
        assert_eq!(back.status, MessageStatus::Read);
        assert_eq!(back.seq, 42);
        assert_eq!(
            back.message.unwrap().conversation.as_deref(),
            Some("halo")
        );
    }

    #[test]
    fn events_are_tagged() {
        let event = WireEvent::Messages {
            chat: "1@g.us".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#"{"type":"messages","chat":"1@g.us"}"#);
        assert_eq!(serde_json::from_str::<WireEvent>(&json).unwrap(), event);
    }
}
