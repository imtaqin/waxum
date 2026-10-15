//! Whatsapp-rust event handling: persistence, webhook payload shaping and message extraction.

use crate::api::shared::sessions::models::SessionStatus;
use crate::state::AppState;
use wacore::proto_helpers::MessageExt;

use super::*;

/// Whether incoming calls are auto-rejected instead of registered for
/// manual `POST .../calls/reject`. Read once from
/// `AUTO_REJECT_INCOMING_CALLS` (default false) and cached — see #114.
pub(super) fn auto_reject_incoming_calls() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("AUTO_REJECT_INCOMING_CALLS")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    })
}

pub(super) async fn handle_event(
    event: std::sync::Arc<wacore::types::events::Event>,
    state: &AppState,
    session_id: &str,
    client: std::sync::Arc<whatsapp_rust::Client>,
) {
    use wacore::types::events::Event;

    let runtime = match state.get_session(session_id) {
        Some(r) => r,
        None => return,
    };

    if !is_current_client(runtime.get_client(), &client)
        && !matches!(event.as_ref(), Event::Messages(_))
    {
        tracing::debug!(
            session_id = %session_id,
            event = %get_event_type(event.as_ref()),
            "ignoring an event from a replaced client"
        );
        return;
    }

    match event.as_ref() {
        Event::PairingQrCode(wacore::types::events::PairingQrCode { code, .. }) => {
            tracing::info!("Session {}: QR code received", session_id);
            runtime.set_qr_codes(vec![code.clone()]);
            runtime.set_status(SessionStatus::WaitingForQr);
            let now = chrono::Utc::now().timestamp();
            runtime.update_pair_state(|ps| {
                ps.last_qr_at = Some(now);
                ps.attempts = ps.attempts.saturating_add(1);
                ps.last_error = None;
            });
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::WaitingForQr, false)
                .await;
        }
        Event::PairingCode(wacore::types::events::PairingCode { code, timeout, .. }) => {
            tracing::info!("Session {}: Pair code received: {}", session_id, code);
            runtime.set_pair_code(Some(code.clone()));
            let now = chrono::Utc::now().timestamp();
            let expires_at = now + timeout.as_secs() as i64;
            runtime.update_pair_state(|ps| {
                ps.last_pair_code_at = Some(now);
                ps.pair_code_expires_at = Some(expires_at);
                ps.last_error = None;
            });
        }
        Event::Connected(_) => {
            tracing::info!("Session {}: Connected", session_id);
            if runtime.reconnecting_for_secs().is_some() {
                crate::metrics::record_session_reconnect(session_id);
            }
            runtime.set_status(SessionStatus::LoggedIn);
            runtime.set_qr_codes(vec![]);
            runtime.set_pair_code(None);
            runtime.clear_pair_state();
            runtime.clear_reconnecting();

            let push_name_str = client.push_name();
            let push_name = if push_name_str.is_empty() {
                None
            } else {
                Some(push_name_str)
            };
            let phone = client.pn().map(|j| j.user.clone());

            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::LoggedIn, true)
                .await;
            let _ = state
                .session_manager()
                .update_session_info(session_id, phone.as_deref(), push_name.as_deref())
                .await;
            let _ = state
                .session_manager()
                .update_last_connected(session_id)
                .await;
        }
        Event::Disconnected(d) => {
            let reason = format!("{}", d.reason);
            let clean = d.reason.is_clean_shutdown();
            if clean {
                tracing::info!(
                    session_id = %session_id,
                    reason = %reason,
                    "socket dropped (clean recycle) — auto-reconnect in flight"
                );
            } else {
                tracing::warn!(
                    session_id = %session_id,
                    reason = %reason,
                    "socket dropped (unexpected) — auto-reconnect in flight"
                );
            }
            runtime.set_status(SessionStatus::Connecting);
            runtime.mark_reconnecting_now();
            crate::metrics::record_session_drop(session_id);
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::Connecting, false)
                .await;
        }
        Event::LoggedOut(logged_out) => {
            let new_chats = crate::api::shared::new_chats::handlers::snapshot_on_logout(
                state,
                session_id,
                &format!("{:?}", logged_out.reason),
            )
            .await;
            let is_lock = matches!(
                logged_out.reason,
                wacore::types::events::ConnectFailureReason::AccountLocked
            );
            if let Some(client) = runtime.get_client() {
                if is_lock {
                    client.set_auto_reconnect(false);
                }
                client.shutdown().await;
            }
            runtime.set_status(SessionStatus::Disconnected);
            runtime.set_client(None);
            runtime.clear_reconnecting();
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::Disconnected, false)
                .await;

            if is_lock {
                let (cooldown_secs, cooldown_until) =
                    runtime.record_lock_cooldown(&state.account_lock_backoff().schedule);
                tracing::warn!(
                    session_id = %session_id,
                    reason = ?logged_out.reason,
                    cooldown_secs,
                    "account_locked, cooling down — auto-reconnect paused until manual reconnect"
                );
                let payload = serde_json::json!({
                    "session_id": session_id,
                    "event": "account_locked",
                    "timestamp": chrono::Utc::now().timestamp(),
                    "data": {
                        "reason": format!("{:?}", logged_out.reason),
                        "cooldown_secs": cooldown_secs,
                        "cooldown_until": cooldown_until,
                        "new_chats": new_chats,
                    },
                });
                if let Ok(payload_str) = serde_json::to_string(&payload) {
                    state
                        .broadcast_to_webhooks(session_id, "account_locked", &payload_str)
                        .await;
                    state
                        .publish_to_nats(session_id, "account_locked", &payload_str)
                        .await;
                    runtime.broadcast_event(payload_str);
                }
                return;
            }

            let should_purge = runtime.record_logout_and_should_purge();
            if !should_purge {
                tracing::warn!(
                    "Session {}: Logged out: {:?} — keeping storage (transient flap)",
                    session_id,
                    logged_out.reason
                );
                return;
            }

            tracing::warn!(
                "Session {}: Logged out: {:?} — repeated, so the stored login is no longer valid; unlinking (session and its data are kept)",
                session_id,
                logged_out.reason
            );
            unlink_session(state, session_id).await;
        }
        Event::IncomingCall(call) => {
            let call_id = call.action.call_id().to_string();
            if auto_reject_incoming_calls() {
                if let Err(e) = client.voip().reject(call).await {
                    tracing::warn!(
                        session_id = %session_id,
                        call_id = %call_id,
                        "AUTO_REJECT_INCOMING_CALLS: failed to reject: {}",
                        e
                    );
                }
            } else if !call_id.is_empty() {
                state.incoming_calls().insert(call_id, *call.clone());
            }
        }
        _ => {}
    }

    persist_contact_event(state, session_id, event.as_ref()).await;

    if let Event::Messages(batch) = event.as_ref() {
        let timestamp = chrono::Utc::now().timestamp();
        let offline = matches!(
            batch.origin,
            wacore::types::events::BatchOrigin::OfflineDrain
        );
        if offline {
            tracing::info!(
                session_id = %session_id,
                count = batch.messages.len(),
                "offline sync: replaying messages received while disconnected"
            );
        }
        for im in batch.messages.iter() {
            let key = format!("{}|{}", im.info.source.chat, im.info.id);
            if !runtime.recent_message_ids.lock().first_sighting(&key) {
                tracing::debug!(
                    session_id = %session_id,
                    message_id = %im.info.id,
                    offline,
                    "dropping redelivered message already forwarded"
                );
                continue;
            }
            crate::api::shared::search::handlers::record_incoming(
                state,
                session_id,
                &im.message,
                &im.info,
            )
            .await;
            let from_phone = resolve_jid_phone(&client, &im.info.source.sender).await;
            let chat_phone = resolve_jid_phone(&client, &im.info.source.chat).await;
            let data = message_event_data(&im.message, &im.info, from_phone, chat_phone);
            let payload_value = serde_json::json!({
                "session_id": session_id,
                "event": "message",
                "timestamp": timestamp,
                "offline": offline,
                "data": data,
            });
            if let Ok(payload) = serde_json::to_string(&payload_value) {
                state
                    .broadcast_to_webhooks(session_id, "message", &payload)
                    .await;
                state.publish_to_nats(session_id, "message", &payload).await;
                runtime.broadcast_event(payload);
                runtime
                    .messages_forwarded
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                runtime
                    .last_message_forwarded_at
                    .store(timestamp, std::sync::atomic::Ordering::Relaxed);
            }
        }
    } else if let Ok(payload) = serde_json::to_string(&event_to_json(event.as_ref(), session_id)) {
        let event_type = get_event_type(event.as_ref());
        state
            .broadcast_to_webhooks(session_id, &event_type, &payload)
            .await;
        state
            .publish_to_nats(session_id, &event_type, &payload)
            .await;
        runtime.broadcast_event(payload);
    }
}

/// `wacore::types::events::Event` is `#[non_exhaustive]`, so the wildcard
/// arm below can never be dropped even once every current variant is named
/// -- the compiler cannot flag a newly added upstream variant here.
/// Bumping the whatsapp-rust pin should include re-diffing this match
/// against the new `Event` definition.
pub(super) fn get_event_type(event: &wacore::types::events::Event) -> String {
    use wacore::types::events::Event;
    match event {
        Event::PairingQrCode { .. } => "qr_code".to_string(),
        Event::PairingCode { .. } => "pair_code".to_string(),
        Event::Connected(_) => "connected".to_string(),
        Event::Disconnected(_) => "disconnected".to_string(),
        Event::LoggedOut(_) => "logged_out".to_string(),
        Event::Messages(_) => "message".to_string(),
        Event::Receipt(_) => "receipt".to_string(),
        Event::Presence(_) => "presence".to_string(),
        Event::ChatPresence(_) => "chat_presence".to_string(),
        Event::GroupUpdate(_) => "group_update".to_string(),
        Event::IncomingCall(_) => "incoming_call".to_string(),
        Event::PictureUpdate(_) => "picture_update".to_string(),
        Event::UserAboutUpdate(_) => "user_about_update".to_string(),
        Event::SelfPushNameUpdated(_) => "push_name_update".to_string(),
        Event::ContactUpdate(_) => "contact_update".to_string(),
        Event::DeviceListUpdate(_) => "device_list_update".to_string(),
        Event::PinUpdate(_) => "pin_update".to_string(),
        Event::MuteUpdate(_) => "mute_update".to_string(),
        Event::ArchiveUpdate(_) => "archive_update".to_string(),
        Event::MarkChatAsReadUpdate(_) => "mark_chat_as_read".to_string(),
        Event::UndecryptableMessage(_) => "undecryptable_message".to_string(),
        Event::ClientOutdated(_) => "client_outdated".to_string(),
        Event::OfflineSyncPreview(_) => "offline_sync_preview".to_string(),
        Event::OfflineSyncCompleted(_) => "offline_sync_completed".to_string(),
        Event::CallLogSync(_) => "call_log_sync".to_string(),
        Event::StreamError(_) => "stream_error".to_string(),
        Event::EncDecryptFailed(_) => "enc_decrypt_failed".to_string(),
        Event::AppStateSyncFailed(_) => "app_state_sync_failed".to_string(),
        Event::BusinessStatusUpdate(_) => "business_status_update".to_string(),
        Event::CallEndedElsewhere(_) => "call_ended_elsewhere".to_string(),
        Event::ClearChatUpdate(_) => "clear_chat_update".to_string(),
        Event::ClientExpirationChanged(_) => "client_expiration_changed".to_string(),
        Event::ConnectFailure(_) => "connect_failure".to_string(),
        Event::ContactNumberChanged(_) => "contact_number_changed".to_string(),
        Event::ContactRemoved(_) => "contact_removed".to_string(),
        Event::ContactSyncRequested(_) => "contact_sync_requested".to_string(),
        Event::ContactUpdated(_) => "contact_updated".to_string(),
        Event::DecryptedPayload(_) => "decrypted_payload".to_string(),
        Event::DeleteChatUpdate(_) => "delete_chat_update".to_string(),
        Event::DeleteMessageForMeUpdate(_) => "delete_message_for_me_update".to_string(),
        Event::DirtyState(_) => "dirty_state".to_string(),
        Event::DisableLinkPreviewsUpdate(_) => "disable_link_previews_update".to_string(),
        Event::DisappearingModeChanged(_) => "disappearing_mode_changed".to_string(),
        Event::FavoriteStickerUpdate(_) => "favorite_sticker_update".to_string(),
        Event::FavoritesUpdate(_) => "favorites_update".to_string(),
        Event::HistorySync(_) => "history_sync".to_string(),
        Event::IdentityChange(_) => "identity_change".to_string(),
        Event::LabelAssociationUpdate(_) => "label_association_update".to_string(),
        Event::LabelEditUpdate(_) => "label_edit_update".to_string(),
        Event::LockChatUpdate(_) => "lock_chat_update".to_string(),
        Event::MessageLabelAssociationUpdate(_) => "message_label_association_update".to_string(),
        Event::MexNotification(_) => "mex_notification".to_string(),
        Event::MissedCall(_) => "missed_call".to_string(),
        Event::NewsletterLiveUpdate(_) => "newsletter_live_update".to_string(),
        Event::Notification(_) => "notification".to_string(),
        Event::OfflineSyncInterrupted(_) => "offline_sync_interrupted".to_string(),
        Event::PairError(_) => "pair_error".to_string(),
        Event::PairingCodeError(_) => "pairing_code_error".to_string(),
        Event::PairingCodeRefresh(_) => "pairing_code_refresh".to_string(),
        Event::PairingQrCodesExhausted(_) => "pairing_qr_codes_exhausted".to_string(),
        Event::PairPasskeyConfirmation(_) => "pair_passkey_confirmation".to_string(),
        Event::PairPasskeyError(_) => "pair_passkey_error".to_string(),
        Event::PairPasskeyRequest(_) => "pair_passkey_request".to_string(),
        Event::PairSuccess(_) => "pair_success".to_string(),
        Event::QrScannedWithoutMultidevice(_) => "qr_scanned_without_multidevice".to_string(),
        Event::QuickReplyUpdate(_) => "quick_reply_update".to_string(),
        Event::RawNode(_) => "raw_node".to_string(),
        Event::RemoveRecentStickerUpdate(_) => "remove_recent_sticker_update".to_string(),
        Event::RetiredPushNameUpdate(_) => "retired_push_name_update".to_string(),
        Event::SentFrame(_) => "sent_frame".to_string(),
        Event::ServerAck(_) => "server_ack".to_string(),
        Event::StarUpdate(_) => "star_update".to_string(),
        Event::StreamReplaced(_) => "stream_replaced".to_string(),
        Event::TemporaryBan(_) => "temporary_ban".to_string(),
        Event::UserStatusMuteUpdate(_) => "user_status_mute_update".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Build the metadata blob a downstream consumer needs to call
/// /sessions/:id/media/download for an inbound media message. Returns null
/// for non-media or text-only messages.
pub(super) fn extract_media_metadata(msg: &waproto::whatsapp::Message) -> serde_json::Value {
    let msg = msg.get_base_message();
    use base64::Engine as _;
    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    if let Some(im) = msg.image_message.as_option() {
        return serde_json::json!({
            "kind": "image",
            "direct_path": im.direct_path,
            "media_key": im.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": im.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": im.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": im.file_length,
            "mimetype": im.mimetype,
            "width": im.width,
            "height": im.height,
        });
    }
    if let Some(vm) = msg.video_message.as_option() {
        return serde_json::json!({
            "kind": "video",
            "direct_path": vm.direct_path,
            "media_key": vm.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": vm.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": vm.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": vm.file_length,
            "mimetype": vm.mimetype,
            "seconds": vm.seconds,
        });
    }
    if let Some(am) = msg.audio_message.as_option() {
        return serde_json::json!({
            "kind": "audio",
            "direct_path": am.direct_path,
            "media_key": am.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": am.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": am.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": am.file_length,
            "mimetype": am.mimetype,
            "seconds": am.seconds,
            "ptt": am.ptt,
        });
    }
    if let Some(dm) = msg.document_message.as_option() {
        return serde_json::json!({
            "kind": "document",
            "direct_path": dm.direct_path,
            "media_key": dm.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": dm.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": dm.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": dm.file_length,
            "mimetype": dm.mimetype,
            "file_name": dm.file_name,
        });
    }
    if let Some(sm) = msg.sticker_message.as_option() {
        return serde_json::json!({
            "kind": "sticker",
            "direct_path": sm.direct_path,
            "media_key": sm.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": sm.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": sm.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": sm.file_length,
            "mimetype": sm.mimetype,
        });
    }
    serde_json::Value::Null
}

/// Typed counterpart to [`extract_media_metadata`] for message-history
/// persistence ([`crate::api::shared::search::handlers::record_incoming`] /
/// `record_outgoing`): the same five media envelopes, but shaped as
/// [`crate::db::messages::MediaPointer`] instead of a webhook JSON blob,
/// and `None` when the message isn't media or is missing a field
/// `/media/download` requires (an incomplete pointer is useless, so a
/// message with one is stored with no media pointer at all rather than
/// a partially-filled one).
///
/// The message is unwrapped first ([`MessageExt::get_base_message`]), because
/// a document sent with a caption arrives inside `document_with_caption_message`
/// and would otherwise be stored without a pointer.
pub(crate) fn extract_media_pointer(
    msg: &waproto::whatsapp::Message,
) -> Option<crate::db::messages::MediaPointer> {
    use base64::Engine as _;
    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    macro_rules! pointer {
        ($m:expr, $kind:literal) => {
            Some(crate::db::messages::MediaPointer {
                media_key: b64($m.media_key.as_ref()?),
                file_sha256: b64($m.file_sha256.as_ref()?),
                file_enc_sha256: b64($m.file_enc_sha256.as_ref()?),
                direct_path: $m.direct_path.clone()?,
                file_length: $m.file_length? as i64,
                media_type: $kind.to_string(),
                mimetype: $m.mimetype.clone().unwrap_or_default(),
            })
        };
    }

    let msg = msg.get_base_message();
    if let Some(im) = msg.image_message.as_option() {
        return pointer!(im, "image");
    }
    if let Some(vm) = msg.video_message.as_option() {
        return pointer!(vm, "video");
    }
    if let Some(am) = msg.audio_message.as_option() {
        return pointer!(am, "audio");
    }
    if let Some(dm) = msg.document_message.as_option() {
        return pointer!(dm, "document");
    }
    if let Some(sm) = msg.sticker_message.as_option() {
        return pointer!(sm, "sticker");
    }
    None
}

/// Extracts location data (lat/lng + optional name/address/url) from a
/// LocationMessage / LiveLocationMessage when present. Returns null otherwise.
pub(super) fn extract_location(msg: &waproto::whatsapp::Message) -> serde_json::Value {
    let msg = msg.get_base_message();
    if let Some(loc) = msg.location_message.as_option() {
        return serde_json::json!({
            "latitude": loc.degrees_latitude,
            "longitude": loc.degrees_longitude,
            "name": loc.name,
            "address": loc.address,
            "url": loc.url,
            "accuracy_meters": loc.accuracy_in_meters,
            "speed_mps": loc.speed_in_mps,
            "is_live": false,
        });
    }
    if let Some(loc) = msg.live_location_message.as_option() {
        return serde_json::json!({
            "latitude": loc.degrees_latitude,
            "longitude": loc.degrees_longitude,
            "accuracy_meters": loc.accuracy_in_meters,
            "speed_mps": loc.speed_in_mps,
            "sequence_number": loc.sequence_number,
            "caption": loc.caption,
            "is_live": true,
        });
    }
    serde_json::Value::Null
}

pub(super) fn message_event_data(
    msg: &waproto::whatsapp::Message,
    info: &wacore::types::message::MessageInfo,
    from_phone: Option<String>,
    chat_phone: Option<String>,
) -> serde_json::Value {
    let (text, caption, message_type, media_mimetype) = extract_message_content(msg);
    let media_meta = extract_media_metadata(msg);
    let location = extract_location(msg);
    let quoted = extract_quoted_context(msg);
    let quoted_message_id = quoted.as_ref().map(|(id, _)| id.clone());
    let quoted_sender_jid = quoted.and_then(|(_, participant)| participant);
    let selection = extract_interactive_selection(msg);
    let native_flow = selection
        .as_ref()
        .filter(|s| s.flow_name.is_some() || s.flow_params.is_some())
        .map(|s| serde_json::json!({ "name": s.flow_name, "params": s.flow_params }));
    serde_json::json!({
        "from": info.source.sender.to_string(),
        "from_phone": from_phone,
        "chat": info.source.chat.to_string(),
        "chat_phone": chat_phone,
        "quoted_message_id": quoted_message_id,
        "quoted_sender_jid": quoted_sender_jid,
        "message_id": info.id.to_string(),
        "timestamp": info.timestamp,
        "is_from_me": info.source.is_from_me,
        "push_name": info.push_name,
        "verified_name": info.verified_name.as_ref().map(|c| format!("{:?}", c)),
        "type": info.r#type,
        "media_type": info.media_type,
        "message_type": message_type,
        "text": text,
        "caption": caption,
        "media_mimetype": media_mimetype,
        "media": media_meta,
        "location": location,
        "selected_id": selection.as_ref().and_then(|s| s.id.clone()),
        "selected_text": selection.as_ref().and_then(|s| s.text.clone()),
        "native_flow": native_flow,
        "is_group": info.source.chat.to_string().ends_with("@g.us"),
        "participant": info.source.sender.to_string(),
    })
}

/// Resolves any JID to its phone number, for the `from_phone`/`chat_phone`
/// webhook fields. Used for both the message sender and the chat JID: for an
/// outgoing message (`is_from_me`), `chat` is the recipient, and on
/// WhatsApp's LID-only privacy accounts that recipient JID is a `@lid`
/// rather than a phone number, so callers otherwise have no phone number to
/// key their own records against.
///
/// A `@lid` JID is looked up in whatsapp-rust's own LID↔PN mapping cache
/// (cache-aside over its persistent backend — no network round trip), which
/// is populated from several passive sources including the `sender_pn`
/// attribute WhatsApp attaches to incoming messages, so it is usually already
/// warm by the time a message arrives. A group JID (`@g.us`) resolves to
/// `None`, same as a `@lid` with no cached mapping yet.
pub(super) async fn resolve_jid_phone(
    client: &whatsapp_rust::Client,
    jid: &wacore_binary::Jid,
) -> Option<String> {
    if jid.is_pn() {
        return Some(jid.user.to_string());
    }
    if !jid.is_lid() {
        return None;
    }
    client
        .get_lid_pn_entry(jid)
        .await
        .ok()
        .flatten()
        .map(|entry| entry.phone_number.to_string())
}

/// Extracts user-visible content from a protobuf Message: best-effort text,
/// optional caption, the high-level type slug, and the media mimetype if any.
/// Shared with the message-history ingestion in
/// [`crate::api::shared::search::handlers`].
///
/// Every `extract_*` helper here first unwraps the ephemeral / view-once /
/// device-sent / document-with-caption wrappers via
/// [`MessageExt::get_base_message`], so a message in a chat with
/// disappearing messages on is read the same as one without.
///
/// A tap on a button or list row is typed by the response kind (see
/// [`extract_interactive_selection`]) with `text` set to the chosen label,
/// falling back to the chosen id, so consumers that only read `text` still
/// see what was picked.
pub(crate) fn extract_message_content(
    msg: &waproto::whatsapp::Message,
) -> (Option<String>, Option<String>, String, Option<String>) {
    let msg = msg.get_base_message();
    let mut text: Option<String> = None;
    let mut caption: Option<String> = None;
    let mut message_type = "unknown".to_string();
    let mut media_mimetype: Option<String> = None;

    if let Some(t) = &msg.conversation {
        if !t.is_empty() {
            text = Some(t.clone());
            message_type = "text".to_string();
        }
    }
    if message_type == "unknown" {
        if let Some(e) = msg.extended_text_message.as_option() {
            text = e.text.clone();
            message_type = "text".to_string();
        } else if let Some(im) = msg.image_message.as_option() {
            caption = im.caption.clone();
            media_mimetype = im.mimetype.clone();
            message_type = "image".to_string();
        } else if let Some(vm) = msg.video_message.as_option() {
            caption = vm.caption.clone();
            media_mimetype = vm.mimetype.clone();
            message_type = "video".to_string();
        } else if let Some(am) = msg.audio_message.as_option() {
            media_mimetype = am.mimetype.clone();
            message_type = if am.ptt.unwrap_or(false) {
                "ptt".to_string()
            } else {
                "audio".to_string()
            };
        } else if let Some(dm) = msg.document_message.as_option() {
            caption = dm.caption.clone();
            text = dm.file_name.clone();
            media_mimetype = dm.mimetype.clone();
            message_type = "document".to_string();
        } else if let Some(sm) = msg.sticker_message.as_option() {
            media_mimetype = sm.mimetype.clone();
            message_type = "sticker".to_string();
        } else if msg.location_message.is_set() || msg.live_location_message.is_set() {
            message_type = "location".to_string();
        } else if msg.contact_message.is_set() {
            message_type = "contact".to_string();
            text = msg
                .contact_message
                .as_option()
                .and_then(|c| c.display_name.clone());
        } else if msg.contacts_array_message.is_set() {
            message_type = "contacts".to_string();
        } else if msg.poll_creation_message.is_set()
            || msg.poll_creation_message_v2.is_set()
            || msg.poll_creation_message_v3.is_set()
        {
            message_type = "poll".to_string();
            text = msg
                .poll_creation_message
                .as_option()
                .and_then(|p| p.name.clone())
                .or_else(|| {
                    msg.poll_creation_message_v2
                        .as_option()
                        .and_then(|p| p.name.clone())
                })
                .or_else(|| {
                    msg.poll_creation_message_v3
                        .as_option()
                        .and_then(|p| p.name.clone())
                });
        } else if msg.poll_update_message.is_set() {
            message_type = "poll_vote".to_string();
        } else if msg.reaction_message.is_set() {
            message_type = "reaction".to_string();
            text = msg
                .reaction_message
                .as_option()
                .and_then(|r| r.text.clone());
        } else if msg.buttons_message.is_set() {
            message_type = "buttons".to_string();
        } else if msg.list_message.is_set() {
            message_type = "list".to_string();
        } else if msg.template_message.is_set() {
            message_type = "template".to_string();
        } else if let Some(selection) = extract_interactive_selection(msg) {
            message_type = selection.kind.to_string();
            text = selection.text.or(selection.id);
        }
    }

    (text, caption, message_type, media_mimetype)
}

/// What the user picked when replying to an interactive message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InteractiveSelection {
    /// `interactive_response`, `buttons_response`, `list_response` or
    /// `template_button_reply` -- also used as the event's `message_type`.
    pub kind: &'static str,
    /// The button id or list row id the sender defined, e.g. `approve`.
    pub id: Option<String>,
    /// The label the user saw and tapped, e.g. `✅ Setuju`.
    pub text: Option<String>,
    /// Native flow name (`quick_reply`, `single_select`, a Flow's name,
    /// ...), only for `interactive_response`.
    pub flow_name: Option<String>,
    /// The native flow `params_json`, parsed. Carries the full submitted
    /// form for a WhatsApp Flow, not just an id.
    pub flow_params: Option<serde_json::Value>,
}

/// Reads the selection out of a reply to buttons, a list, a template
/// button or a native-flow (quick-reply / single-select / Flow) message.
/// `None` for every other message kind.
///
/// Native-flow replies carry their choice in `params_json`, a JSON string
/// whose `id` is the button or row id the sender defined; when it has no
/// `id` (a submitted Flow form), the parsed object is still returned in
/// `flow_params`.
pub(crate) fn extract_interactive_selection(
    msg: &waproto::whatsapp::Message,
) -> Option<InteractiveSelection> {
    let msg = msg.get_base_message();
    if let Some(r) = msg.interactive_response_message.as_option() {
        let native = match &r.interactive_response_message {
            Some(
                waproto::whatsapp::message::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(n),
            ) => Some(n.as_ref()),
            _ => None,
        };
        let flow_params = native
            .and_then(|n| n.params_json.as_deref())
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
        let id = flow_params
            .as_ref()
            .and_then(|p| p.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let text = r
            .body
            .as_option()
            .and_then(|b| b.text.clone())
            .filter(|t| !t.is_empty());
        return Some(InteractiveSelection {
            kind: "interactive_response",
            id,
            text,
            flow_name: native.and_then(|n| n.name.clone()),
            flow_params,
        });
    }
    if let Some(r) = msg.buttons_response_message.as_option() {
        let text = match &r.response {
            Some(
                waproto::whatsapp::message::buttons_response_message::Response::SelectedDisplayText(
                    t,
                ),
            ) => Some(t.clone()),
            _ => None,
        };
        return Some(InteractiveSelection {
            kind: "buttons_response",
            id: r.selected_button_id.clone(),
            text,
            flow_name: None,
            flow_params: None,
        });
    }
    if let Some(r) = msg.list_response_message.as_option() {
        return Some(InteractiveSelection {
            kind: "list_response",
            id: r
                .single_select_reply
                .as_option()
                .and_then(|s| s.selected_row_id.clone()),
            text: r.title.clone(),
            flow_name: None,
            flow_params: None,
        });
    }
    if let Some(r) = msg.template_button_reply_message.as_option() {
        return Some(InteractiveSelection {
            kind: "template_button_reply",
            id: r.selected_id.clone(),
            text: r.selected_display_text.clone(),
            flow_name: None,
            flow_params: None,
        });
    }
    None
}

/// Extracts the WhatsApp reply/quote linkage (`ContextInfo.stanzaId` +
/// `.participant`) from whichever content submessage the message carries.
/// `None` when the message is not a reply. `context_info` lives per
/// content type in the protobuf, not on `Message` itself, so this checks
/// every content type that the wire format actually attaches one to
/// (confirmed against `contextInfo` fields in the whatsapp-rust proto,
/// not merely the subset `extract_message_content` above branches on).
pub(crate) fn extract_quoted_context(
    msg: &waproto::whatsapp::Message,
) -> Option<(String, Option<String>)> {
    let msg = msg.get_base_message();
    let ctx = msg
        .extended_text_message
        .as_option()
        .and_then(|m| m.context_info.as_option())
        .or_else(|| {
            msg.image_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.video_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.audio_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.document_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.sticker_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.contact_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.contacts_array_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.location_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.live_location_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.poll_creation_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.poll_creation_message_v2
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.poll_creation_message_v3
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.buttons_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.buttons_response_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.list_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.list_response_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.interactive_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.interactive_response_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.template_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.template_button_reply_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.order_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            msg.product_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })?;
    let stanza_id = ctx.stanza_id.clone()?;
    Some((stanza_id, ctx.participant.clone()))
}

pub(super) async fn persist_contact_event(
    state: &AppState,
    session_id: &str,
    event: &wacore::types::events::Event,
) {
    use wacore::types::events::Event;
    let store = crate::db::contacts::ContactStore::new(state.session_manager().pool());

    if let Event::Messages(batch) = event {
        for im in batch.messages.iter() {
            let info = &im.info;
            if info.source.is_from_me {
                continue;
            }
            let sender = &info.source.sender;
            let jid_str = sender.to_string();
            let mut push_name = None::<String>;
            if !info.push_name.is_empty() {
                push_name = Some(info.push_name.to_string());
            }
            let mut business_name = None::<String>;
            if let Some(vn) = info.verified_name.as_ref() {
                let s = format!("{:?}", vn);
                if !s.is_empty() && s != "None" {
                    business_name = Some(s);
                }
            }
            let mut phone_str = None::<String>;
            if sender.server == wacore_binary::jid::SERVER_JID {
                phone_str = Some(sender.user.to_string());
            }
            let upsert = crate::db::contacts::ContactUpsert {
                session_id,
                jid: &jid_str,
                phone: phone_str.as_deref(),
                push_name: push_name.as_deref(),
                business_name: business_name.as_deref(),
                source: "message",
                ..Default::default()
            };
            if let Err(e) = store.upsert(&upsert).await {
                tracing::warn!(
                    "contacts: upsert failed for {}/{}: {}",
                    session_id,
                    jid_str,
                    e
                );
            }
        }
        return;
    }

    let mut upsert = crate::db::contacts::ContactUpsert {
        session_id,
        ..Default::default()
    };
    let jid_str;
    let mut phone_str = None::<String>;
    let mut lid_str = None::<String>;
    let mut full_name = None::<String>;
    let mut first_name = None::<String>;
    let push_name = None::<String>;
    let business_name = None::<String>;
    let source: &str;

    match event {
        Event::ContactUpdate(u) => {
            jid_str = u.jid.to_string();
            if let Some(name) = u.action.full_name.as_deref() {
                if !name.is_empty() {
                    full_name = Some(name.to_string());
                }
            }
            if let Some(name) = u.action.first_name.as_deref() {
                if !name.is_empty() {
                    first_name = Some(name.to_string());
                }
            }
            if let Some(lid) = u.action.lid_jid.as_deref() {
                if !lid.is_empty() {
                    lid_str = Some(lid.to_string());
                }
            }
            if u.jid.server == wacore_binary::jid::SERVER_JID {
                phone_str = Some(u.jid.user.to_string());
            }
            source = if u.from_full_sync {
                "appstate_sync"
            } else {
                "appstate"
            };
        }
        Event::ContactUpdated(u) => {
            jid_str = u.jid.to_string();
            if u.jid.server == wacore_binary::jid::SERVER_JID {
                phone_str = Some(u.jid.user.to_string());
            }
            source = "notification";
        }
        _ => return,
    }

    upsert.jid = &jid_str;
    upsert.phone = phone_str.as_deref();
    upsert.lid_jid = lid_str.as_deref();
    upsert.full_name = full_name.as_deref();
    upsert.first_name = first_name.as_deref();
    upsert.push_name = push_name.as_deref();
    upsert.business_name = business_name.as_deref();
    upsert.source = source;

    if let Err(e) = store.upsert(&upsert).await {
        tracing::warn!(
            "contacts: upsert failed for {}/{}: {}",
            session_id,
            jid_str,
            e
        );
    }
}

pub(super) fn event_to_json(
    event: &wacore::types::events::Event,
    session_id: &str,
) -> serde_json::Value {
    use wacore::types::events::Event;

    let event_type = get_event_type(event);
    let timestamp = chrono::Utc::now().timestamp();

    let data = match event {
        Event::Messages(batch) => match batch.first() {
            Some(im) => message_event_data(&im.message, &im.info, None, None),
            None => serde_json::json!({}),
        },
        Event::Receipt(receipt) => {
            serde_json::json!({
                "receipt": format!("{:?}", receipt),
            })
        }
        Event::Presence(presence) => {
            serde_json::json!({
                "jid": presence.from.to_string(),
                "available": presence.status == wacore::types::presence::PresenceStatus::Available,
                "last_seen": presence.last_seen,
            })
        }
        Event::ChatPresence(presence) => {
            serde_json::json!({
                "chat": presence.source.chat.to_string(),
                "sender": presence.source.sender.to_string(),
                "state": format!("{:?}", presence.state),
            })
        }
        Event::GroupUpdate(update) => {
            serde_json::json!({
                "group": update.group_jid.to_string(),
                "update": format!("{:?}", update.action),
            })
        }
        Event::IncomingCall(call) => {
            serde_json::json!({
                "from": call.from.to_string(),
                "stanza_id": call.stanza_id,
                "call_id": call.action.call_id().to_string(),
                "call_creator": call.action.call_creator().to_string(),
                "notify": call.notify,
                "platform": call.platform,
                "version": call.version,
                "timestamp": call.timestamp.map(|t| t.timestamp()),
                "offline": call.offline,
                "action": format!("{:?}", call.action),
            })
        }
        Event::PictureUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "author": update.author.as_ref().map(|j| j.to_string()).unwrap_or_default(),
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::UserAboutUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "status": update.status,
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::SelfPushNameUpdated(update) => {
            serde_json::json!({
                "old_name": update.old_name,
                "new_name": update.new_name,
                "from_server": update.from_server,
            })
        }
        Event::ContactUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
            })
        }
        Event::DeviceListUpdate(update) => {
            serde_json::json!({
                "user": update.user.to_string(),
                "update_type": format!("{:?}", update.update_type),
            })
        }
        Event::PinUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "pinned": update.action.pinned,
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::MuteUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "muted": update.action.muted,
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::ArchiveUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "archived": update.action.archived,
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::MarkChatAsReadUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "timestamp": update.timestamp.timestamp(),
                "from_full_sync": update.from_full_sync,
            })
        }
        Event::UndecryptableMessage(msg) => {
            serde_json::json!({
                "info": format!("{:?}", msg),
            })
        }
        Event::ClientOutdated(info) => {
            serde_json::json!({
                "info": format!("{:?}", info),
            })
        }
        Event::OfflineSyncPreview(preview) => {
            serde_json::json!({
                "info": format!("{:?}", preview),
            })
        }
        Event::OfflineSyncCompleted(completed) => {
            serde_json::json!({
                "info": format!("{:?}", completed),
            })
        }
        Event::CallLogSync(sync) => {
            serde_json::json!({
                "call_creator_jid": sync.call_creator_jid.to_string(),
                "call_id": sync.call_id,
                "from_me": sync.from_me,
                "timestamp": sync.timestamp.timestamp(),
                "from_full_sync": sync.from_full_sync,
                "record": format!("{:?}", sync.record),
            })
        }
        Event::StreamError(err) => {
            serde_json::json!({
                "code": err.code,
            })
        }
        Event::EncDecryptFailed(failed) => {
            serde_json::json!({
                "chat": failed.info.source.chat.to_string(),
                "sender": failed.info.source.sender.to_string(),
                "message_id": failed.info.id,
                "enc_index": failed.enc_index,
                "enc_type": failed.enc_type,
                "reason": format!("{:?}", failed.reason),
            })
        }
        Event::PairingQrCode(qr) => {
            serde_json::json!({
                "code": qr.code,
                "timeout_seconds": qr.timeout.as_secs(),
            })
        }
        Event::PairingCode(pair) => {
            serde_json::json!({
                "code": pair.code,
                "timeout_seconds": pair.timeout.as_secs(),
            })
        }
        Event::PairingCodeError(failed) => {
            serde_json::json!({
                "error": failed.error,
                "rejection": failed.rejection.map(|r| format!("{r:?}")),
                "throttled": failed.rejection.is_some_and(|r| r.is_throttled()),
                "retry_after_seconds": failed.backoff.map(|d| d.as_secs()),
            })
        }
        Event::PairError(failed) => {
            serde_json::json!({
                "jid": failed.id.to_string(),
                "platform": failed.platform,
                "error": failed.error,
            })
        }
        _ => serde_json::json!({}),
    };

    serde_json::json!({
        "session_id": session_id,
        "event": event_type,
        "timestamp": timestamp,
        "data": data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    mod media_pointer {
        use super::super::*;
        use waproto::buffa::MessageField;
        use waproto::whatsapp::message as wm;
        use waproto::whatsapp::Message;

        fn document() -> wm::DocumentMessage {
            nx!(wm::DocumentMessage {
                media_key: Some(vec![1; 32]),
                file_sha256: Some(vec![2; 32]),
                file_enc_sha256: Some(vec![3; 32]),
                direct_path: Some("/v/doc.enc".to_string()),
                file_length: Some(4096),
                mimetype: Some(
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                        .to_string(),
                ),
                ..Default::default()
            })
        }

        #[test]
        fn plain_document_has_a_pointer() {
            let msg = nx!(Message {
                document_message: MessageField::some(document()),
                ..Default::default()
            });
            let ptr = extract_media_pointer(&msg).expect("pointer");
            assert_eq!(ptr.media_type, "document");
            assert_eq!(ptr.file_length, 4096);
        }

        #[test]
        fn document_with_caption_wrapper_has_a_pointer() {
            let inner = nx!(Message {
                document_message: MessageField::some(document()),
                ..Default::default()
            });
            let msg = nx!(Message {
                document_with_caption_message: MessageField::some(nx!(wm::FutureProofMessage {
                    message: MessageField::some(inner),
                })),
                ..Default::default()
            });
            let ptr = extract_media_pointer(&msg).expect("pointer");
            assert_eq!(ptr.media_type, "document");
            assert_eq!(ptr.direct_path, "/v/doc.enc");
        }
    }

    mod interactive_replies {
        use super::super::*;
        use waproto::buffa::MessageField;
        use waproto::whatsapp::message as wm;
        use waproto::whatsapp::{ContextInfo, Message};

        fn quoted(id: &str) -> MessageField<ContextInfo> {
            MessageField::some(nx!(ContextInfo {
                stanza_id: Some(id.to_string()),
                ..Default::default()
            }))
        }

        fn quick_reply_tap() -> Message {
            nx!(Message {
                interactive_response_message: MessageField::some(nx!(wm::InteractiveResponseMessage {
                    body: MessageField::some(nx!(wm::interactive_response_message::Body {
                        text: Some("✅ Setuju".to_string()),
                        ..Default::default()
                    })),
                    context_info: quoted("3EB0D7A57AA45E7D72FE2B"),
                    interactive_response_message: Some(
                        wm::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(
                            Box::new(nx!(wm::interactive_response_message::NativeFlowResponseMessage {
                                name: Some("quick_reply".to_string()),
                                params_json: Some(r#"{"id":"approve"}"#.to_string()),
                                version: Some(3),
                            })),
                        ),
                    ),
                })),
                ..Default::default()
            })
        }

        fn info() -> wacore::types::message::MessageInfo {
            wacore::types::message::MessageInfo {
                source: wacore::types::message::MessageSource {
                    chat: "628123456789@s.whatsapp.net".parse().unwrap(),
                    sender: "628123456789@s.whatsapp.net".parse().unwrap(),
                    ..Default::default()
                },
                ..Default::default()
            }
        }

        #[test]
        fn quick_reply_tap_reports_the_chosen_button() {
            let msg = quick_reply_tap();
            let (text, _, message_type, _) = extract_message_content(&msg);
            assert_eq!(message_type, "interactive_response");
            assert_eq!(text.as_deref(), Some("✅ Setuju"));

            let data = message_event_data(&msg, &info(), None, None);
            assert_eq!(data["message_type"], "interactive_response");
            assert_eq!(data["selected_id"], "approve");
            assert_eq!(data["selected_text"], "✅ Setuju");
            assert_eq!(data["text"], "✅ Setuju");
            assert_eq!(data["quoted_message_id"], "3EB0D7A57AA45E7D72FE2B");
            assert_eq!(data["native_flow"]["name"], "quick_reply");
            assert_eq!(data["native_flow"]["params"]["id"], "approve");
        }

        #[test]
        fn list_row_pick_reports_the_row_id_and_title() {
            let msg = nx!(Message {
                list_response_message: MessageField::some(nx!(wm::ListResponseMessage {
                    title: Some("Hari ini".to_string()),
                    list_type: Some(wm::list_response_message::ListType::SINGLE_SELECT),
                    single_select_reply: MessageField::some(nx!(
                        wm::list_response_message::SingleSelectReply {
                            selected_row_id: Some("rep_today".to_string()),
                        }
                    ),),
                    context_info: quoted("3EB096CDC66E16DACCA06C"),
                    ..Default::default()
                })),
                ..Default::default()
            });
            let data = message_event_data(&msg, &info(), None, None);
            assert_eq!(data["message_type"], "list_response");
            assert_eq!(data["selected_id"], "rep_today");
            assert_eq!(data["selected_text"], "Hari ini");
            assert_eq!(data["text"], "Hari ini");
            assert_eq!(data["quoted_message_id"], "3EB096CDC66E16DACCA06C");
            assert_eq!(data["native_flow"], serde_json::Value::Null);
        }

        #[test]
        fn legacy_buttons_and_template_buttons_report_the_selection() {
            let buttons = nx!(Message {
                buttons_response_message: MessageField::some(nx!(wm::ButtonsResponseMessage {
                    selected_button_id: Some("reject".to_string()),
                    r#type: Some(wm::buttons_response_message::Type::DISPLAY_TEXT),
                    response: Some(wm::buttons_response_message::Response::SelectedDisplayText(
                        "❌ Tolak".to_string(),
                    )),
                    ..Default::default()
                })),
                ..Default::default()
            });
            let sel = extract_interactive_selection(&buttons).unwrap();
            assert_eq!(sel.kind, "buttons_response");
            assert_eq!(sel.id.as_deref(), Some("reject"));
            assert_eq!(sel.text.as_deref(), Some("❌ Tolak"));

            let template = nx!(Message {
                template_button_reply_message: MessageField::some(nx!(
                    wm::TemplateButtonReplyMessage {
                        selected_id: Some("track".to_string()),
                        selected_display_text: Some("Track order".to_string()),
                        selected_index: Some(0),
                        ..Default::default()
                    }
                )),
                ..Default::default()
            });
            let (text, _, message_type, _) = extract_message_content(&template);
            assert_eq!(message_type, "template_button_reply");
            assert_eq!(text.as_deref(), Some("Track order"));
        }

        #[test]
        fn a_flow_submission_keeps_the_whole_form_and_falls_back_to_id_for_text() {
            let msg = nx!(Message {
                interactive_response_message: MessageField::some(nx!(wm::InteractiveResponseMessage {
                    interactive_response_message: Some(
                        wm::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(
                            Box::new(nx!(wm::interactive_response_message::NativeFlowResponseMessage {
                                name: Some("flow".to_string()),
                                params_json: Some(
                                    r#"{"flow_token":"t1","slot":"09:00"}"#.to_string(),
                                ),
                                version: Some(3),
                            })),
                        ),
                    ),
                    ..Default::default()
                })),
                ..Default::default()
            });
            let data = message_event_data(&msg, &info(), None, None);
            assert_eq!(data["message_type"], "interactive_response");
            assert_eq!(data["selected_id"], serde_json::Value::Null);
            assert_eq!(data["native_flow"]["params"]["slot"], "09:00");

            let text_only = nx!(Message {
                list_response_message: MessageField::some(nx!(wm::ListResponseMessage {
                    single_select_reply: MessageField::some(nx!(
                        wm::list_response_message::SingleSelectReply {
                            selected_row_id: Some("row_9".to_string()),
                        }
                    ),),
                    ..Default::default()
                })),
                ..Default::default()
            });
            let (text, _, _, _) = extract_message_content(&text_only);
            assert_eq!(text.as_deref(), Some("row_9"));
        }

        #[test]
        fn replies_in_a_disappearing_messages_chat_are_unwrapped() {
            let wrapped = nx!(Message {
                ephemeral_message: MessageField::some(nx!(wm::FutureProofMessage {
                    message: MessageField::some(quick_reply_tap()),
                })),
                ..Default::default()
            });
            let data = message_event_data(&wrapped, &info(), None, None);
            assert_eq!(data["message_type"], "interactive_response");
            assert_eq!(data["selected_id"], "approve");
            assert_eq!(data["quoted_message_id"], "3EB0D7A57AA45E7D72FE2B");

            let plain_text = nx!(Message {
                ephemeral_message: MessageField::some(nx!(wm::FutureProofMessage {
                    message: MessageField::some(nx!(Message {
                        conversation: Some("halo".to_string()),
                        ..Default::default()
                    })),
                })),
                ..Default::default()
            });
            let (text, _, message_type, _) = extract_message_content(&plain_text);
            assert_eq!(message_type, "text");
            assert_eq!(text.as_deref(), Some("halo"));
        }

        #[test]
        fn ordinary_messages_have_no_selection() {
            let msg = nx!(Message {
                conversation: Some("hi".to_string()),
                ..Default::default()
            });
            assert!(extract_interactive_selection(&msg).is_none());
            let data = message_event_data(&msg, &info(), None, None);
            assert_eq!(data["selected_id"], serde_json::Value::Null);
            assert_eq!(data["selected_text"], serde_json::Value::Null);
            assert_eq!(data["native_flow"], serde_json::Value::Null);
        }
    }

    #[test]
    fn message_event_data_carries_a_chat_phone_alongside_the_chat_jid() {
        let info = wacore::types::message::MessageInfo {
            source: wacore::types::message::MessageSource {
                chat: "628123456789@s.whatsapp.net".parse().unwrap(),
                sender: "628000000000@s.whatsapp.net".parse().unwrap(),
                is_from_me: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let msg = waproto::whatsapp::Message::default();

        let data = message_event_data(
            &msg,
            &info,
            Some("628000000000".to_string()),
            Some("628123456789".to_string()),
        );

        assert_eq!(data["chat"], "628123456789@s.whatsapp.net");
        assert_eq!(data["chat_phone"], "628123456789");
        assert_eq!(data["from_phone"], "628000000000");
    }

    #[test]
    fn message_event_data_chat_phone_is_null_when_unresolved() {
        let info = wacore::types::message::MessageInfo {
            source: wacore::types::message::MessageSource {
                chat: "100000012345678@lid".parse().unwrap(),
                sender: "628000000000@s.whatsapp.net".parse().unwrap(),
                is_from_me: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let msg = waproto::whatsapp::Message::default();

        let data = message_event_data(&msg, &info, Some("628000000000".to_string()), None);

        assert_eq!(data["chat"], "100000012345678@lid");
        assert!(data["chat_phone"].is_null());
    }

    #[test]
    fn new_event_variants_map_to_their_webhook_names() {
        use wacore::types::events::{CallLogSync, EncDecryptFailed, Event, StreamError};

        let stream_error =
            Event::StreamError(StreamError::builder().code("429".to_string()).build());
        assert_eq!(get_event_type(&stream_error), "stream_error");

        let call_log = Event::CallLogSync(
            CallLogSync::builder()
                .call_creator_jid("12345@s.whatsapp.net".parse().unwrap())
                .call_id("call-1".to_string())
                .from_me(true)
                .timestamp(chrono::Utc::now())
                .record(Box::new(waproto::whatsapp::CallLogRecord::default()))
                .from_full_sync(false)
                .build(),
        );
        assert_eq!(get_event_type(&call_log), "call_log_sync");

        let enc_failed = Event::EncDecryptFailed(
            EncDecryptFailed::builder()
                .info(std::sync::Arc::new(
                    wacore::types::message::MessageInfo::default(),
                ))
                .enc_index(1)
                .enc_type(std::borrow::Cow::Borrowed("skmsg"))
                .reason(wacore::types::events::EncDecryptFailureReason::NoSession)
                .build(),
        );
        assert_eq!(get_event_type(&enc_failed), "enc_decrypt_failed");
    }

    #[test]
    fn new_event_variants_serialize_with_event_name_and_data() {
        use wacore::types::events::{CallLogSync, EncDecryptFailed, Event, StreamError};

        let stream_error =
            Event::StreamError(StreamError::builder().code("429".to_string()).build());
        let json = event_to_json(&stream_error, "sess");
        assert_eq!(json["event"], "stream_error");
        assert_eq!(json["session_id"], "sess");
        assert_eq!(json["data"]["code"], "429");

        let call_log = Event::CallLogSync(
            CallLogSync::builder()
                .call_creator_jid("12345@s.whatsapp.net".parse().unwrap())
                .call_id("call-1".to_string())
                .from_me(true)
                .timestamp(chrono::Utc::now())
                .record(Box::new(waproto::whatsapp::CallLogRecord::default()))
                .from_full_sync(true)
                .build(),
        );
        let json = event_to_json(&call_log, "sess");
        assert_eq!(json["event"], "call_log_sync");
        assert_eq!(json["data"]["call_id"], "call-1");
        assert_eq!(json["data"]["from_me"], true);
        assert_eq!(json["data"]["from_full_sync"], true);

        let enc_failed = Event::EncDecryptFailed(
            EncDecryptFailed::builder()
                .info(std::sync::Arc::new(
                    wacore::types::message::MessageInfo::default(),
                ))
                .enc_index(2)
                .enc_type(std::borrow::Cow::Borrowed("pkmsg"))
                .reason(wacore::types::events::EncDecryptFailureReason::BadMac)
                .build(),
        );
        let json = event_to_json(&enc_failed, "sess");
        assert_eq!(json["event"], "enc_decrypt_failed");
        assert_eq!(json["data"]["enc_index"], 2);
        assert_eq!(json["data"]["enc_type"], "pkmsg");
        assert_eq!(json["data"]["reason"], "BadMac");
    }

    #[test]
    fn qr_and_pair_code_events_carry_the_code_in_their_data() {
        use wacore::types::events::{Event, PairingCode, PairingQrCode};

        let qr = Event::PairingQrCode(
            PairingQrCode::builder()
                .code("2@abc123,def456==".to_string())
                .timeout(std::time::Duration::from_secs(60))
                .build(),
        );
        let json = event_to_json(&qr, "sess");
        assert_eq!(json["event"], "qr_code");
        assert_eq!(json["data"]["code"], "2@abc123,def456==");
        assert_eq!(json["data"]["timeout_seconds"], 60);

        let pair = Event::PairingCode(
            PairingCode::builder()
                .code("ABCD1234".to_string())
                .timeout(std::time::Duration::from_secs(180))
                .build(),
        );
        let json = event_to_json(&pair, "sess");
        assert_eq!(json["event"], "pair_code");
        assert_eq!(json["data"]["code"], "ABCD1234");
        assert_eq!(json["data"]["timeout_seconds"], 180);
    }

    #[test]
    fn pairing_code_error_carries_the_reason() {
        use wacore::pair_code::PairCodeRejection;
        use wacore::types::events::{Event, PairingCodeError};

        let refused = Event::PairingCodeError(
            PairingCodeError::builder()
                .rejection(PairCodeRejection::RateOverlimit)
                .backoff(std::time::Duration::from_secs(90))
                .error("server refused the pair code request: 429".to_string())
                .build(),
        );
        let json = event_to_json(&refused, "sess");
        assert_eq!(json["event"], "pairing_code_error");
        assert_eq!(
            json["data"]["error"],
            "server refused the pair code request: 429"
        );
        assert_eq!(json["data"]["rejection"], "RateOverlimit");
        assert_eq!(json["data"]["throttled"], true);
        assert_eq!(json["data"]["retry_after_seconds"], 90);

        let local = Event::PairingCodeError(
            PairingCodeError::builder()
                .error("not connected".to_string())
                .build(),
        );
        let json = event_to_json(&local, "sess");
        assert_eq!(json["data"]["error"], "not connected");
        assert!(json["data"]["rejection"].is_null());
        assert_eq!(json["data"]["throttled"], false);
        assert!(json["data"]["retry_after_seconds"].is_null());
    }

    #[test]
    fn zip_round_trips_nested_directory() {
        let src = tempfile::tempdir().expect("src tempdir");
        std::fs::write(src.path().join("device.json"), b"top-level file").unwrap();
        std::fs::create_dir(src.path().join("keys")).unwrap();
        std::fs::write(src.path().join("keys/identity.bin"), b"nested file").unwrap();

        let zip_bytes = zip_directory(src.path().to_str().unwrap()).expect("zip");

        let dst = tempfile::tempdir().expect("dst tempdir");
        let dst_path = dst.path().join("restored");
        unzip_directory(dst_path.to_str().unwrap(), &zip_bytes).expect("unzip");

        assert_eq!(
            std::fs::read(dst_path.join("device.json")).unwrap(),
            b"top-level file"
        );
        assert_eq!(
            std::fs::read(dst_path.join("keys/identity.bin")).unwrap(),
            b"nested file"
        );
    }

    #[test]
    fn unzip_rejects_path_traversal() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("../../etc/passwd", options).unwrap();
            std::io::Write::write_all(&mut writer, b"pwned").unwrap();
            writer.finish().unwrap();
        }

        let dst = tempfile::tempdir().expect("dst tempdir");
        let dst_path = dst.path().join("restored");
        let result = unzip_directory(dst_path.to_str().unwrap(), &buf.into_inner());
        assert!(result.is_err(), "path traversal entry must be rejected");
    }

    #[test]
    fn unzip_rejects_too_many_entries() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            for i in 0..=MAX_IMPORT_ENTRIES {
                writer.start_file(format!("f{i}"), options).unwrap();
            }
            writer.finish().unwrap();
        }

        let dst = tempfile::tempdir().expect("dst tempdir");
        let dst_path = dst.path().join("restored");
        let result = unzip_directory(dst_path.to_str().unwrap(), &buf.into_inner());
        let err = result.expect_err("entry-count limit must be enforced");
        assert!(err.to_string().contains("entries"), "{err}");
    }

    /// The filler is all zeros -- highly compressible, so this zip stays
    /// small on disk. A real zip bomb needs the *decompressed* size
    /// checked, not the compressed size; that's exactly what this proves.
    #[test]
    fn unzip_rejects_entry_over_the_per_file_size_cap() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("huge.bin", options).unwrap();
            let chunk = vec![0u8; 8 * 1024 * 1024];
            let chunks_needed = (MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES / chunk.len() as u64) + 1;
            for _ in 0..chunks_needed {
                std::io::Write::write_all(&mut writer, &chunk).unwrap();
            }
            writer.finish().unwrap();
        }

        let dst = tempfile::tempdir().expect("dst tempdir");
        let dst_path = dst.path().join("restored");
        let result = unzip_directory(dst_path.to_str().unwrap(), &buf.into_inner());
        let err = result.expect_err("per-entry uncompressed size limit must be enforced");
        assert!(err.to_string().contains("per-file"), "{err}");
    }
}
