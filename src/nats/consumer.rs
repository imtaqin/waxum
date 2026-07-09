use std::time::Duration;

use async_nats::jetstream::{self, consumer::PullConsumer, AckKind};
use futures::StreamExt;
use waproto::buffa::{Enumeration, MessageField};

use crate::handlers::messages::{get_client, get_media_data, parse_jid};
use crate::state::AppState;

use super::models::{OutboundCommand, SendResult};

/// Start the NATS outbound message consumer.
/// Subscribes to `wa.send.>` and dispatches messages to WhatsApp clients.
pub async fn start_consumer(
    jetstream: jetstream::Context,
    state: AppState,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let stream = jetstream.get_stream("WA_SEND").await?;

    let consumer: PullConsumer = stream
        .get_or_create_consumer(
            "wa-send-worker",
            jetstream::consumer::pull::Config {
                durable_name: Some("wa-send-worker".into()),
                filter_subject: "wa.send.>".into(),
                ack_policy: jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                max_deliver: 3,
                ..Default::default()
            },
        )
        .await?;

    let handle = tokio::spawn(async move {
        loop {
            match consumer.messages().await {
                Ok(mut messages) => {
                    while let Some(Ok(msg)) = messages.next().await {
                        let session_id = extract_session_id(&msg.subject);
                        let state = state.clone();
                        let payload = msg.payload.to_vec();

                        tokio::spawn(async move {
                            let result =
                                process_outbound_command(&state, &session_id, &payload).await;

                            match &result {
                                Ok(send_result) => {
                                    // Publish result to events stream
                                    if let Ok(result_json) = serde_json::to_string(send_result) {
                                        state
                                            .publish_to_nats(
                                                &session_id,
                                                "send_result",
                                                &result_json,
                                            )
                                            .await;
                                    }
                                    let _ = msg.ack().await;
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to process outbound command for session {}: {}",
                                        session_id,
                                        e
                                    );
                                    // NAK with delay for retry
                                    let _ = msg
                                        .ack_with(AckKind::Nak(Some(Duration::from_secs(5))))
                                        .await;
                                }
                            }
                        });
                    }
                }
                Err(e) => {
                    tracing::error!("NATS consumer stream error: {}, reconnecting in 5s", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });

    Ok(handle)
}

fn extract_session_id(subject: &str) -> String {
    // subject format: wa.send.{session_id}
    subject
        .strip_prefix("wa.send.")
        .unwrap_or("unknown")
        .to_string()
}

async fn process_outbound_command(
    state: &AppState,
    session_id: &str,
    payload: &[u8],
) -> anyhow::Result<SendResult> {
    let command: OutboundCommand =
        serde_json::from_slice(payload).map_err(|e| anyhow::anyhow!("Invalid command: {}", e))?;

    let request_id = command.request_id();

    let client =
        get_client(state, session_id).map_err(|e| anyhow::anyhow!("Session error: {}", e))?;

    let result = dispatch_command(&client, command).await;

    match result {
        Ok(message_id) => Ok(SendResult {
            request_id,
            success: true,
            message_id: Some(message_id),
            error: None,
            timestamp: chrono::Utc::now().timestamp(),
        }),
        Err(e) => Err(e),
    }
}

async fn dispatch_command(
    client: &std::sync::Arc<whatsapp_rust::Client>,
    command: OutboundCommand,
) -> anyhow::Result<String> {
    match command {
        OutboundCommand::Text { to, text, .. } => {
            let to_jid = parse_jid(&to)?;
            let message = waproto::whatsapp::Message {
                extended_text_message: MessageField::some(
                    waproto::whatsapp::message::ExtendedTextMessage {
                        text: Some(text),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Image {
            to, image, caption, ..
        } => {
            let to_jid = parse_jid(&to)?;
            let (data, mimetype) = get_media_data(&image).await?;
            let upload = client
                .upload(
                    data.clone(),
                    wacore::download::MediaType::Image,
                    Default::default(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let message = waproto::whatsapp::Message {
                image_message: MessageField::some(waproto::whatsapp::message::ImageMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key.to_vec()),
                    file_sha256: Some(upload.file_sha256.to_vec()),
                    file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                    file_length: Some(data.len() as u64),
                    mimetype: Some(mimetype),
                    caption,
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Video {
            to, video, caption, ..
        } => {
            let to_jid = parse_jid(&to)?;
            let (data, mimetype) = get_media_data(&video).await?;
            let upload = client
                .upload(
                    data.clone(),
                    wacore::download::MediaType::Video,
                    Default::default(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let message = waproto::whatsapp::Message {
                video_message: MessageField::some(waproto::whatsapp::message::VideoMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key.to_vec()),
                    file_sha256: Some(upload.file_sha256.to_vec()),
                    file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                    file_length: Some(data.len() as u64),
                    mimetype: Some(mimetype),
                    caption,
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Audio { to, audio, ptt, .. } => {
            let to_jid = parse_jid(&to)?;
            let (data, mimetype) = get_media_data(&audio).await?;
            let upload = client
                .upload(
                    data.clone(),
                    wacore::download::MediaType::Audio,
                    Default::default(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let message = waproto::whatsapp::Message {
                audio_message: MessageField::some(waproto::whatsapp::message::AudioMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key.to_vec()),
                    file_sha256: Some(upload.file_sha256.to_vec()),
                    file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                    file_length: Some(data.len() as u64),
                    mimetype: Some(mimetype),
                    ptt: Some(ptt),
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Document {
            to,
            document,
            filename,
            caption,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let (data, mimetype) = get_media_data(&document).await?;
            let upload = client
                .upload(
                    data.clone(),
                    wacore::download::MediaType::Document,
                    Default::default(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let message = waproto::whatsapp::Message {
                document_message: MessageField::some(waproto::whatsapp::message::DocumentMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key.to_vec()),
                    file_sha256: Some(upload.file_sha256.to_vec()),
                    file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                    file_length: Some(data.len() as u64),
                    mimetype: Some(mimetype),
                    file_name: Some(filename),
                    caption,
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Sticker { to, sticker, .. } => {
            let to_jid = parse_jid(&to)?;
            let (data, mimetype) = get_media_data(&sticker).await?;
            let upload = client
                .upload(
                    data.clone(),
                    wacore::download::MediaType::Sticker,
                    Default::default(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let message = waproto::whatsapp::Message {
                sticker_message: MessageField::some(waproto::whatsapp::message::StickerMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key.to_vec()),
                    file_sha256: Some(upload.file_sha256.to_vec()),
                    file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                    file_length: Some(data.len() as u64),
                    mimetype: Some(mimetype),
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Location {
            to,
            latitude,
            longitude,
            name,
            address,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let message = waproto::whatsapp::Message {
                location_message: MessageField::some(waproto::whatsapp::message::LocationMessage {
                    degrees_latitude: Some(latitude),
                    degrees_longitude: Some(longitude),
                    name,
                    address,
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Contact { to, contact, .. } => {
            let to_jid = parse_jid(&to)?;
            let vcard = format!(
                "BEGIN:VCARD\nVERSION:3.0\nFN:{}\n{}END:VCARD",
                contact.display_name,
                contact
                    .phones
                    .iter()
                    .map(|p| format!("TEL;type={}:{}\n", p.phone_type, p.number))
                    .collect::<String>()
            );
            let message = waproto::whatsapp::Message {
                contact_message: MessageField::some(waproto::whatsapp::message::ContactMessage {
                    display_name: Some(contact.display_name),
                    vcard: Some(vcard),
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Reaction {
            to,
            message_id,
            emoji,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let message = waproto::whatsapp::Message {
                reaction_message: MessageField::some(waproto::whatsapp::message::ReactionMessage {
                    key: Some(waproto::whatsapp::MessageKey {
                        remote_jid: Some(to.clone()),
                        id: Some(message_id),
                        from_me: Some(false),
                        ..Default::default()
                    })
                    .into(),
                    text: Some(emoji),
                    sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Poll {
            to,
            name,
            options,
            selectable_count,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let opts: Vec<waproto::whatsapp::message::poll_creation_message::Option> = options
                .into_iter()
                .map(
                    |n| waproto::whatsapp::message::poll_creation_message::Option {
                        option_name: Some(n),
                        ..Default::default()
                    },
                )
                .collect();
            let message = waproto::whatsapp::Message {
                poll_creation_message: MessageField::some(
                    waproto::whatsapp::message::PollCreationMessage {
                        name: Some(name),
                        options: opts,
                        selectable_options_count: Some(selectable_count),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Buttons {
            to,
            content_text,
            footer,
            buttons,
            header_text,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let btns: Vec<waproto::whatsapp::message::buttons_message::Button> = buttons
                .into_iter()
                .map(|b| waproto::whatsapp::message::buttons_message::Button {
                    button_id: Some(b.button_id),
                    button_text: Some(
                        waproto::whatsapp::message::buttons_message::button::ButtonText {
                            display_text: Some(b.display_text),
                        },
                    )
                    .into(),
                    r#type: Some(
                        waproto::whatsapp::message::buttons_message::button::Type::from_i32(1)
                            .unwrap_or_default(),
                    ),
                    ..Default::default()
                })
                .collect();
            let header = header_text.map(waproto::whatsapp::message::buttons_message::Header::Text);
            let message = waproto::whatsapp::Message {
                buttons_message: MessageField::some(waproto::whatsapp::message::ButtonsMessage {
                    content_text: Some(content_text),
                    footer_text: footer,
                    buttons: btns,
                    header_type: header.as_ref().map(|_| {
                        waproto::whatsapp::message::buttons_message::HeaderType::from_i32(2)
                            .unwrap_or_default()
                    }),
                    header,
                    ..Default::default()
                }),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::List {
            to,
            title,
            description,
            button_text,
            sections,
            footer,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let sections_json: Vec<serde_json::Value> = sections
                .iter()
                .map(|s| {
                    let rows: Vec<serde_json::Value> = s
                        .rows
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "id": r.row_id,
                                "title": r.title,
                                "description": r.description.as_deref().unwrap_or("")
                            })
                        })
                        .collect();
                    serde_json::json!({ "title": s.title, "rows": rows })
                })
                .collect();
            let list_params = serde_json::json!({
                "title": title,
                "button": button_text,
                "sections": sections_json
            });
            let native_flow =
                waproto::whatsapp::message::interactive_message::NativeFlowMessage {
                    buttons: vec![
                    waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                        name: Some("single_select".to_string()),
                        button_params_json: Some(list_params.to_string()),
                    },
                ],
                    ..Default::default()
                };
            let message = waproto::whatsapp::Message {
                interactive_message: MessageField::some(
                    waproto::whatsapp::message::InteractiveMessage {
                        body: Some(waproto::whatsapp::message::interactive_message::Body {
                            text: Some(description),
                        }).into(),
                        footer: footer.map(|f| {
                            waproto::whatsapp::message::interactive_message::Footer {
                                text: Some(f),
                                ..Default::default()
                            }
                        }).into(),
                        interactive_message: Some(
                            waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(Box::new(native_flow)),
                        ),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Interactive {
            to,
            body_text,
            footer_text,
            buttons,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let btns: Vec<waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton> = buttons
                .into_iter()
                .map(|b| waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                    name: Some(b.name),
                    button_params_json: Some(b.button_params_json),
                })
                .collect();
            let native_flow = waproto::whatsapp::message::interactive_message::NativeFlowMessage {
                buttons: btns,
                ..Default::default()
            };
            let message = waproto::whatsapp::Message {
                interactive_message: MessageField::some(
                    waproto::whatsapp::message::InteractiveMessage {
                        body: Some(waproto::whatsapp::message::interactive_message::Body {
                            text: Some(body_text),
                        }).into(),
                        footer: footer_text.map(|f| {
                            waproto::whatsapp::message::interactive_message::Footer {
                                text: Some(f),
                                ..Default::default()
                            }
                        }).into(),
                        interactive_message: Some(
                            waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(Box::new(native_flow)),
                        ),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            };
            client
                .send_message(to_jid, message)
                .await
                .map(|r| r.message_id)
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Revoke {
            to,
            message_id,
            original_sender,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let revoke_type = match original_sender {
                Some(sender) => {
                    let sender_jid = parse_jid(&sender)?;
                    whatsapp_rust::RevokeType::Admin {
                        original_sender: sender_jid,
                    }
                }
                None => whatsapp_rust::RevokeType::Sender,
            };
            client
                .revoke_message(to_jid, &message_id, revoke_type)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(message_id)
        }

        OutboundCommand::Edit {
            to,
            message_id,
            text,
            ..
        } => {
            let to_jid = parse_jid(&to)?;
            let edit_msg = waproto::whatsapp::Message {
                extended_text_message: MessageField::some(
                    waproto::whatsapp::message::ExtendedTextMessage {
                        text: Some(text),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            };
            client
                .edit_message(to_jid, &message_id, edit_msg)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }

        OutboundCommand::Read {
            chat_jid,
            sender,
            message_ids,
            ..
        } => {
            let chat = parse_jid(&chat_jid)?;
            let sender_jid = match &sender {
                Some(s) => Some(parse_jid(s)?),
                None => None,
            };
            let id_refs: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
            client
                .mark_as_read(&chat, sender_jid.as_ref(), &id_refs)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok("read".to_string())
        }
    }
}
