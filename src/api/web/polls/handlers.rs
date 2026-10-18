use axum::{
    extract::{Path, State},
    Json,
};
use wacore::proto_helpers::MessageExt;
use wacore_binary::jid::{Jid, Server};
use waproto::buffa::MessageField;
use waproto::whatsapp as wa;
use whatsapp_rust::PollVoteCiphertext;

use crate::api::shared::messages::handlers::{chat_store, get_client, parse_jid, stored_message};
use crate::api::web::polls::models::{
    PollOptionTally, PollResultsResponse, PollVoteRequest, PollVoteResponse,
};
use crate::error::ApiError;
use crate::state::AppState;

/// How far back in a chat to look for votes on a poll.
const VOTE_SCAN: i64 = 5000;

struct PollInfo {
    question: String,
    options: Vec<String>,
    selectable: u32,
    secret: Vec<u8>,
    creator: Jid,
}

fn creation(m: &wa::Message) -> Option<&wa::message::PollCreationMessage> {
    m.poll_creation_message
        .as_option()
        .or_else(|| m.poll_creation_message_v2.as_option())
        .or_else(|| m.poll_creation_message_v3.as_option())
}

async fn poll_info(
    state: &AppState,
    session_id: &str,
    client: &whatsapp_rust::Client,
    chat: &Jid,
    message_id: &str,
) -> Result<PollInfo, ApiError> {
    let stored = stored_message(state, session_id, chat, message_id)
        .await
        .ok_or_else(|| ApiError::MessageNotFound(message_id.to_string()))?;
    let proto = stored
        .message
        .as_deref()
        .ok_or_else(|| ApiError::MessageNotFound(message_id.to_string()))?;
    let base = proto.get_base_message();
    let poll = creation(base)
        .ok_or_else(|| ApiError::BadRequest(format!("{message_id} is not a poll")))?;
    let secret = proto
        .message_context_info
        .as_option()
        .or_else(|| base.message_context_info.as_option())
        .and_then(|c| c.message_secret.clone())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::BadRequest(
                "the poll carries no message secret, so votes cannot be read".into(),
            )
        })?;
    let creator = if stored.from_me {
        client.pn().ok_or(ApiError::NotConnected)?.to_non_ad()
    } else {
        stored.sender_jid.to_non_ad()
    };
    Ok(PollInfo {
        question: poll.name.clone().unwrap_or_default(),
        options: poll
            .options
            .iter()
            .filter_map(|o| o.option_name.clone())
            .collect(),
        selectable: poll.selectable_options_count.unwrap_or(0),
        secret,
        creator,
    })
}

/// The identity this account votes under: its LID for LID-addressed
/// polls, its phone JID otherwise (as WhatsApp Web does).
fn own_voter(client: &whatsapp_rust::Client, creator: &Jid) -> Option<Jid> {
    let pn = client.pn()?.to_non_ad();
    if creator.server == Server::Lid {
        Some(client.lid().map(|l| l.to_non_ad()).unwrap_or(pn))
    } else {
        Some(pn)
    }
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/polls/{chat}/{message_id}",
    tag = "polls",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("chat" = String, Path, description = "Chat the poll is in"),
        ("message_id" = String, Path, description = "Poll message id")
    ),
    responses(
        (status = 200, description = "Current tally, one entry per option", body = PollResultsResponse),
        (status = 404, description = "Poll not in the chat store"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn poll_results(
    State(state): State<AppState>,
    Path((session_id, chat, message_id)): Path<(String, String, String)>,
) -> Result<Json<PollResultsResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&chat)?;
    let info = poll_info(&state, &session_id, &client, &chat, &message_id).await?;
    let store = chat_store(&state, &session_id).ok_or(ApiError::NotConnected)?;
    let me = own_voter(&client, &info.creator);

    let messages = store
        .messages(&chat, None, VOTE_SCAN)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut voters: Vec<Jid> = Vec::new();
    let mut ciphertexts: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for m in messages.iter().rev() {
        let Some(proto) = m.message.as_deref() else {
            continue;
        };
        let Some(update) = proto.get_base_message().poll_update_message.as_option() else {
            continue;
        };
        let targets_poll = update
            .poll_creation_message_key
            .as_option()
            .and_then(|k| k.id.as_deref())
            == Some(message_id.as_str());
        let Some(vote) = update.vote.as_option() else {
            continue;
        };
        let (Some(payload), Some(iv)) = (vote.enc_payload.clone(), vote.enc_iv.clone()) else {
            continue;
        };
        if !targets_poll {
            continue;
        }
        let voter = if m.from_me {
            match &me {
                Some(me) => me.clone(),
                None => continue,
            }
        } else {
            m.sender_jid.to_non_ad()
        };
        voters.push(voter);
        ciphertexts.push((payload, iv));
    }
    let votes: Vec<(&Jid, PollVoteCiphertext<'_>)> = voters
        .iter()
        .zip(ciphertexts.iter())
        .map(|(voter, (payload, iv))| {
            (
                voter,
                PollVoteCiphertext {
                    enc_payload: payload,
                    enc_iv: iv,
                },
            )
        })
        .collect();

    let tally = client
        .polls()
        .aggregate_votes(
            &info.options,
            &votes,
            &info.secret,
            &message_id,
            &info.creator,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mine: Vec<String> = me
        .map(|me| {
            let me_user = me.user.to_string();
            let me_alt = client.lid().map(|l| l.user.to_string());
            tally
                .iter()
                .filter(|o| {
                    o.voters.iter().any(|v| {
                        let user = v.split('@').next().unwrap_or(v);
                        user == me_user || me_alt.as_deref() == Some(user)
                    })
                })
                .map(|o| o.name.clone())
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(PollResultsResponse {
        question: info.question,
        selectable_count: info.selectable,
        options: tally
            .into_iter()
            .map(|o| PollOptionTally {
                name: o.name,
                voters: o.voters,
            })
            .collect(),
        my_votes: mine,
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/polls/{chat}/{message_id}/vote",
    tag = "polls",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("chat" = String, Path, description = "Chat the poll is in"),
        ("message_id" = String, Path, description = "Poll message id")
    ),
    request_body = PollVoteRequest,
    responses(
        (status = 200, description = "Vote sent", body = PollVoteResponse),
        (status = 400, description = "Unknown option, or too many options for this poll"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn vote(
    State(state): State<AppState>,
    Path((session_id, chat, message_id)): Path<(String, String, String)>,
    Json(request): Json<PollVoteRequest>,
) -> Result<Json<PollVoteResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&chat)?;
    let info = poll_info(&state, &session_id, &client, &chat, &message_id).await?;
    if let Some(unknown) = request.options.iter().find(|o| !info.options.contains(o)) {
        return Err(ApiError::BadRequest(format!(
            "{unknown} is not an option of this poll"
        )));
    }
    if info.selectable > 0 && request.options.len() > info.selectable as usize {
        return Err(ApiError::BadRequest(format!(
            "this poll allows {} option(s)",
            info.selectable
        )));
    }
    let voter = own_voter(&client, &info.creator).ok_or(ApiError::NotConnected)?;
    let hashes: Vec<Vec<u8>> = request
        .options
        .iter()
        .map(|o| wacore::poll::compute_option_hash(o).to_vec())
        .collect();
    let (payload, iv) = wacore::poll::encrypt_poll_vote_with_secret(
        &hashes,
        &info.secret,
        &message_id,
        &info.creator.to_string(),
        &voter.to_string(),
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let own_pn = client.pn().ok_or(ApiError::NotConnected)?;
    let creator_is_me = own_pn.to_non_ad().user == info.creator.user
        || client.lid().is_some_and(|l| l.user == info.creator.user);
    let message = nx!(wa::Message {
        poll_update_message: MessageField::some(nx!(wa::message::PollUpdateMessage {
            poll_creation_message_key: MessageField::some(nx!(wa::MessageKey {
                remote_jid: Some(chat.to_string()),
                from_me: Some(creator_is_me),
                id: Some(message_id.clone()),
                participant: (chat.server == Server::Group).then(|| info.creator.to_string()),
            })),
            vote: MessageField::some(nx!(wa::message::PollEncValue {
                enc_payload: Some(payload),
                enc_iv: Some(iv.to_vec()),
            })),
            metadata: MessageField::none(),
            sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
        })),
        ..Default::default()
    });
    let sent = client
        .send_message(chat.clone(), message.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        &state,
        &session_id,
        &chat,
        &message,
        sent.message_id.as_str(),
    )
    .await;
    Ok(Json(PollVoteResponse {
        message_id: sent.message_id.to_string(),
    }))
}
