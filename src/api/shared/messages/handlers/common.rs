//! Helpers shared by every send handler: client and JID resolution, media
//! decoding, Cloud API response shaping and the auto-presence typing delay.

use base64::Engine;
use wacore_binary::jid::Jid;

use crate::api::shared::messages::models::*;
use crate::error::ApiError;
use crate::state::AppState;

pub(super) fn cloud_message_response(resp: serde_json::Value, to: String) -> MessageResponse {
    let message_id = resp
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|a| a.first())
        .and_then(|m| m.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to,
    }
}

pub(crate) fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_live_client().ok_or(ApiError::NotConnected)
}

pub(crate) fn parse_jid(jid_str: &str) -> Result<Jid, ApiError> {
    let trimmed = jid_str.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidJid(jid_str.to_string()));
    }
    if trimmed.contains('@') {
        trimmed
            .parse()
            .map_err(|_| ApiError::InvalidJid(trimmed.to_string()))
    } else {
        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return Err(ApiError::InvalidJid(trimmed.to_string()));
        }
        Ok(Jid::pn(&digits))
    }
}

/// Resolve a recipient JID to its actual deliverable address.
///
/// WhatsApp has migrated most contacts to LID-only privacy mode: sending
/// to legacy `phone@s.whatsapp.net` is accepted by the server but never
/// delivered. For any `@s.whatsapp.net` recipient we query the contact
/// directory and substitute the LID when one exists. Already-resolved
/// JIDs (`@lid`, `@g.us`, etc.) and unknown contacts pass through
/// unchanged.
pub(crate) async fn resolve_recipient_jid(
    client: std::sync::Arc<whatsapp_rust::Client>,
    jid: Jid,
) -> Jid {
    use wacore_binary::jid::SERVER_JID;
    if jid.server != SERVER_JID {
        return jid;
    }
    let probe = vec![jid.clone()];
    match do_get_user_info_lite(client, probe).await {
        Ok(map) => map
            .get(&jid)
            .and_then(|info| info.lid.clone())
            .unwrap_or(jid),
        Err(_) => jid,
    }
}

pub(super) async fn do_get_user_info_lite(
    client: std::sync::Arc<whatsapp_rust::Client>,
    jids: Vec<Jid>,
) -> Result<std::collections::HashMap<Jid, whatsapp_rust::UserInfo>, ()> {
    struct AssertSend<F>(F);
    unsafe impl<F: std::future::Future> Send for AssertSend<F> {}
    impl<F: std::future::Future> std::future::Future for AssertSend<F> {
        type Output = F::Output;
        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
        }
    }
    AssertSend(async move { client.contacts().get_user_info(&jids).await.map_err(|_| ()) }).await
}

pub(crate) async fn get_media_data(media: &MediaData) -> Result<(Vec<u8>, String), ApiError> {
    match media {
        MediaData::Url { url } => {
            crate::net_guard::validate_public_url(url)
                .await
                .map_err(ApiError::BadRequest)?;

            let response = crate::net_guard::safe_http_client()
                .get(url)
                .send()
                .await
                .map_err(|e| ApiError::BadRequest(format!("Failed to fetch URL: {}", e)))?;

            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();

            let data = response
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("Failed to read response: {}", e)))?
                .to_vec();

            Ok((data, content_type))
        }
        MediaData::Base64 { data, mimetype } => {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| ApiError::BadRequest(format!("Invalid base64: {}", e)))?;
            Ok((decoded, mimetype.clone()))
        }
        MediaData::Uploaded { mimetype: _, .. } => Err(ApiError::BadRequest(
            "Pre-uploaded media not supported in this context".to_string(),
        )),
    }
}

pub(super) fn build_vcard(contact: &ContactCard) -> String {
    let mut vcard = String::new();
    vcard.push_str("BEGIN:VCARD\n");
    vcard.push_str("VERSION:3.0\n");
    vcard.push_str(&format!("FN:{}\n", contact.display_name));
    vcard.push_str(&format!("N:;{};;;\n", contact.display_name));

    if let Some(org) = &contact.organization {
        vcard.push_str(&format!("ORG:{}\n", org));
    }

    for phone in &contact.phones {
        vcard.push_str(&format!("TEL;type={}:{}\n", phone.phone_type, phone.number));
    }

    vcard.push_str("END:VCARD");
    vcard
}

/// Whether a send simulates a composing indicator first. Read once from
/// `AUTO_PRESENCE_ON_SEND` (default false) and cached — see #114: gives the
/// "typing…" UX on every message/media send without a separate
/// `chatstate/typing` call in front of it. Best-effort: a failed presence
/// send is swallowed and never blocks the actual message send.
pub(super) fn auto_presence_on_send() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("AUTO_PRESENCE_ON_SEND")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    })
}

/// How long the composing indicator is held before the message goes out, in
/// milliseconds. Read once from `AUTO_PRESENCE_DELAY_MS` (default 1200 ms) so
/// the recipient's chat UI has time to render "typing..." rather than a
/// flicker. Only consulted when `AUTO_PRESENCE_ON_SEND` is enabled.
pub(super) fn auto_presence_delay_ms() -> u64 {
    static DELAY: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *DELAY.get_or_init(|| {
        parse_presence_delay(std::env::var("AUTO_PRESENCE_DELAY_MS").ok().as_deref())
    })
}

/// Parses an `AUTO_PRESENCE_DELAY_MS` value. Missing or malformed input falls
/// back to the default, and the result is capped so a typo cannot stall sends.
pub(super) fn parse_presence_delay(raw: Option<&str>) -> u64 {
    const DEFAULT_MS: u64 = 1200;
    const MAX_MS: u64 = 10_000;
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_MS)
        .min(MAX_MS)
}

/// Shows the composing indicator to `jid` and holds it for
/// `AUTO_PRESENCE_DELAY_MS` before returning, so the caller's message lands
/// while the recipient still sees the indicator. Best-effort: a failed
/// presence send skips the delay and never blocks the message.
pub(super) async fn show_typing_before_send(client: &whatsapp_rust::Client, jid: &Jid) {
    if client.chatstate().send_composing(jid).await.is_err() {
        return;
    }
    let delay_ms = auto_presence_delay_ms();
    if delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_presence_delay_defaults_on_missing_or_malformed_input() {
        for raw in [
            None,
            Some(""),
            Some("  "),
            Some("soon"),
            Some("-5"),
            Some("1.5"),
        ] {
            assert_eq!(parse_presence_delay(raw), 1200);
        }
    }

    #[test]
    fn parse_presence_delay_accepts_milliseconds() {
        assert_eq!(parse_presence_delay(Some("0")), 0);
        assert_eq!(parse_presence_delay(Some("750")), 750);
        assert_eq!(parse_presence_delay(Some(" 1500 ")), 1500);
    }

    #[test]
    fn parse_presence_delay_caps_absurd_values() {
        assert_eq!(parse_presence_delay(Some("600000")), 10_000);
        assert_eq!(parse_presence_delay(Some(&u64::MAX.to_string())), 10_000);
    }
}
