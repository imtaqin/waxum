//! Meta webhook contract: GET verification handshake, POST signature
//! check, and payload normalization.
//!
//! This is a different contract from waxum's own outbound webhook
//! signing (`X-Webhook-Signature`, HMAC over `{timestamp}.{body}`, see
//! [`crate::state`]): Meta signs each delivery with
//! `X-Hub-Signature-256: sha256=<hex>` over the raw request body, keyed
//! by the app secret, with no timestamp component and no replay window
//! of its own.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verifies Meta's `X-Hub-Signature-256` header against the raw request
/// body. `header` is the full header value, e.g. `sha256=abcd...`.
pub fn verify_signature(app_secret: &str, raw_body: &[u8], header: Option<&str>) -> bool {
    let Some(header) = header else {
        return false;
    };
    let Some(hex_sig) = header.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(app_secret.as_bytes()) else {
        return false;
    };
    mac.update(raw_body);
    mac.verify_slice(&expected).is_ok()
}

/// Answers the `GET .../cloud/webhook` verification handshake Meta sends
/// when the webhook URL is registered: returns `hub.challenge` when
/// `hub.mode == "subscribe"` and `hub.verify_token` matches, `None`
/// otherwise (the caller responds 403 on `None`).
pub fn verify_challenge<'a>(
    verify_token: &str,
    mode: Option<&str>,
    token: Option<&'a str>,
    challenge: Option<&'a str>,
) -> Option<&'a str> {
    if mode == Some("subscribe") && token == Some(verify_token) {
        challenge
    } else {
        None
    }
}

/// Normalizes one Meta webhook delivery (`entry[].changes[].value`) into
/// waxum's own `message` event envelope, matching the shape
/// `handlers::sessions::message_event_data` produces for whatsapp-rust
/// sessions -- `from`, `from_phone`, `chat`, `chat_phone`, `message_id`,
/// `is_from_me`, `push_name`, `message_type`, `text`, `caption`,
/// `media`, `location`, `is_group`, `quoted_message_id`,
/// `quoted_sender_jid` -- so a consumer's webhook receiver doesn't need
/// a provider-specific code path.
///
/// Only inbound `messages[]` entries produce an event; `statuses[]`
/// (delivered/read/failed) delivery-status updates are not yet mapped to
/// a webhook event of their own in this phase.
pub fn normalize_messages(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let Some(entries) = payload.get("entry").and_then(|v| v.as_array()) else {
        return out;
    };
    for entry in entries {
        let Some(changes) = entry.get("changes").and_then(|v| v.as_array()) else {
            continue;
        };
        for change in changes {
            let Some(value) = change.get("value") else {
                continue;
            };
            let contacts = value.get("contacts").and_then(|v| v.as_array());
            let Some(messages) = value.get("messages").and_then(|v| v.as_array()) else {
                continue;
            };
            for msg in messages {
                out.push(normalize_one_message(value, msg, contacts));
            }
        }
    }
    out
}

fn normalize_one_message(
    value: &serde_json::Value,
    msg: &serde_json::Value,
    contacts: Option<&Vec<serde_json::Value>>,
) -> serde_json::Value {
    let from = msg
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let message_id = msg
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let msg_type = msg
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let timestamp = msg
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let push_name = contacts
        .and_then(|cs| {
            cs.iter()
                .find(|c| c.get("wa_id").and_then(|w| w.as_str()) == Some(&from))
        })
        .and_then(|c| c.get("profile"))
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let text = msg
        .get("text")
        .and_then(|t| t.get("body"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let caption = msg
        .get(&msg_type)
        .and_then(|m| m.get("caption"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let media = ["image", "video", "audio", "document", "sticker"]
        .contains(&msg_type.as_str())
        .then(|| msg.get(&msg_type).cloned())
        .flatten();
    let location = (msg_type == "location")
        .then(|| msg.get("location").cloned())
        .flatten();
    let quoted_message_id = msg
        .get("context")
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let quoted_sender_jid = msg
        .get("context")
        .and_then(|c| c.get("from"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let phone_number_id = value
        .get("metadata")
        .and_then(|m| m.get("phone_number_id"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    serde_json::json!({
        "from": from,
        "from_phone": from,
        "chat": phone_number_id,
        "chat_phone": value
            .get("metadata")
            .and_then(|m| m.get("display_phone_number"))
            .and_then(|v| v.as_str()),
        "quoted_message_id": quoted_message_id,
        "quoted_sender_jid": quoted_sender_jid,
        "message_id": message_id,
        "timestamp": timestamp,
        "is_from_me": false,
        "push_name": push_name,
        "verified_name": null,
        "type": msg_type,
        "media_type": null,
        "message_type": msg_type,
        "text": text,
        "caption": caption,
        "media": media,
        "location": location,
        "is_group": false,
        "participant": from,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_a_correctly_signed_body() {
        let secret = "shh";
        let body = br#"{"hello":"world"}"#;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());
        let header = format!("sha256={sig}");
        assert!(verify_signature(secret, body, Some(&header)));
    }

    #[test]
    fn rejects_a_bad_signature() {
        let body = br#"{"hello":"world"}"#;
        assert!(!verify_signature("shh", body, Some("sha256=deadbeef")));
        assert!(!verify_signature("shh", body, None));
    }

    #[test]
    fn challenge_only_echoes_on_matching_token_and_subscribe_mode() {
        assert_eq!(
            verify_challenge("tok", Some("subscribe"), Some("tok"), Some("chal")),
            Some("chal")
        );
        assert_eq!(
            verify_challenge("tok", Some("subscribe"), Some("wrong"), Some("chal")),
            None
        );
        assert_eq!(
            verify_challenge("tok", Some("unsubscribe"), Some("tok"), Some("chal")),
            None
        );
    }

    #[test]
    fn normalizes_an_inbound_text_message_with_reply_context() {
        let payload = serde_json::json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "metadata": {"phone_number_id": "106540", "display_phone_number": "15551234567"},
                        "contacts": [{"wa_id": "15559876543", "profile": {"name": "Ada"}}],
                        "messages": [{
                            "from": "15559876543",
                            "id": "wamid.ABC",
                            "timestamp": "1700000000",
                            "type": "text",
                            "text": {"body": "hi"},
                            "context": {"id": "wamid.ORIGINAL", "from": "15551234567"}
                        }]
                    }
                }]
            }]
        });
        let events = normalize_messages(&payload);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e["from"], "15559876543");
        assert_eq!(e["text"], "hi");
        assert_eq!(e["message_type"], "text");
        assert_eq!(e["is_from_me"], false);
        assert_eq!(e["is_group"], false);
        assert_eq!(e["quoted_message_id"], "wamid.ORIGINAL");
        assert_eq!(e["push_name"], "Ada");
    }
}
