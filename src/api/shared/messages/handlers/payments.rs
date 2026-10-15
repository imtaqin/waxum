//! Order, invoice and payment sends and payment-request actions.

use axum::{
    extract::{Path, State},
    Json,
};
use waproto::buffa::{Enumeration, MessageField};

use super::common::*;
use crate::api::shared::messages::models::*;
use crate::api::shared::schedule::models::SendResponse;
use crate::error::ApiError;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/order",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendOrderRequest,
    responses(
        (status = 200, description = "Order message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_order(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendOrderRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "order",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_order(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `order` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_order(
    state: &AppState,
    session_id: &str,
    request: SendOrderRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let status = request.status.as_deref().and_then(|s| match s {
        "inquiry" => Some(waproto::whatsapp::message::order_message::OrderStatus::INQUIRY),
        "accepted" => Some(waproto::whatsapp::message::order_message::OrderStatus::ACCEPTED),
        "declined" => Some(waproto::whatsapp::message::order_message::OrderStatus::DECLINED),
        _ => None,
    });

    let message = nx!(waproto::whatsapp::Message {
        order_message: MessageField::some(nx!(waproto::whatsapp::message::OrderMessage {
            order_id: Some(request.order_id),
            item_count: request.item_count,
            status,
            message: request.message,
            order_title: request.order_title,
            seller_jid: request.seller_jid,
            token: request.token,
            total_amount1000: request.total_amount_1000,
            total_currency_code: request.total_currency_code,
            ..Default::default()
        })),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/invoice",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendInvoiceRequest,
    responses(
        (status = 200, description = "Invoice message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_invoice(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendInvoiceRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "invoice",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_invoice(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `invoice` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_invoice(
    state: &AppState,
    session_id: &str,
    request: SendInvoiceRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let attachment_type = request.attachment_type.as_deref().and_then(|t| match t {
        "image" => Some(waproto::whatsapp::message::invoice_message::AttachmentType::IMAGE),
        "pdf" => Some(waproto::whatsapp::message::invoice_message::AttachmentType::PDF),
        _ => None,
    });

    let message = nx!(waproto::whatsapp::Message {
        invoice_message: MessageField::some(nx!(waproto::whatsapp::message::InvoiceMessage {
            note: request.note,
            token: request.token,
            attachment_type,
            attachment_mimetype: request.attachment_mimetype,
            ..Default::default()
        })),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/payment-invite",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendPaymentInviteRequest,
    responses(
        (status = 200, description = "Payment invite sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_payment_invite(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPaymentInviteRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "payment-invite",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_payment_invite(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `payment-invite` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_payment_invite(
    state: &AppState,
    session_id: &str,
    request: SendPaymentInviteRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let message = nx!(waproto::whatsapp::Message {
        payment_invite_message: MessageField::some(nx!(
            waproto::whatsapp::message::PaymentInviteMessage {
                service_type: request.service_type.and_then(
                    waproto::whatsapp::message::payment_invite_message::ServiceType::from_i32,
                ),
                ..Default::default()
            }
        ),),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/send-payment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendPaymentRequest,
    responses(
        (status = 200, description = "Payment sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_payment(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPaymentRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "send-payment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_payment(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `send-payment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_payment(
    state: &AppState,
    session_id: &str,
    request: SendPaymentRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let note_message = request.note.map(|text| {
        nx!(waproto::whatsapp::Message {
            extended_text_message: MessageField::some(nx!(
                waproto::whatsapp::message::ExtendedTextMessage {
                    text: Some(text),
                    ..Default::default()
                }
            ),),
            ..Default::default()
        })
    });

    let request_message_key = request.request_message_id.map(|id| {
        nx!(waproto::whatsapp::MessageKey {
            remote_jid: Some(request.to.clone()),
            id: Some(id),
            from_me: Some(false),
            ..Default::default()
        })
    });

    let message = nx!(waproto::whatsapp::Message {
        send_payment_message: MessageField::some(nx!(
            waproto::whatsapp::message::SendPaymentMessage {
                note_message: note_message.into(),
                request_message_key: request_message_key.into(),
                transaction_data: request.transaction_data,
                ..Default::default()
            }
        )),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/request-payment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RequestPaymentRequest,
    responses(
        (status = 200, description = "Payment request sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn request_payment(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RequestPaymentRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "request-payment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_request_payment(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `request-payment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_request_payment(
    state: &AppState,
    session_id: &str,
    request: RequestPaymentRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let note_message = request.note.map(|text| {
        nx!(waproto::whatsapp::Message {
            extended_text_message: MessageField::some(nx!(
                waproto::whatsapp::message::ExtendedTextMessage {
                    text: Some(text),
                    ..Default::default()
                }
            ),),
            ..Default::default()
        })
    });

    let message = nx!(waproto::whatsapp::Message {
        request_payment_message: MessageField::some(nx!(
            waproto::whatsapp::message::RequestPaymentMessage {
                note_message: note_message.into(),
                currency_code_iso4217: Some(request.currency_code),
                amount1000: Some(request.amount1000),
                request_from: Some(request.to.clone()),
                expiry_timestamp: request.expiry_timestamp,
                ..Default::default()
            }
        ),),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/cancel-payment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = CancelPaymentRequestRequest,
    responses(
        (status = 200, description = "Payment request cancelled", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn cancel_payment_request(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<CancelPaymentRequestRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "cancel-payment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_cancel_payment_request(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `cancel-payment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_cancel_payment_request(
    state: &AppState,
    session_id: &str,
    request: CancelPaymentRequestRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let message = nx!(waproto::whatsapp::Message {
        cancel_payment_request_message: MessageField::some(nx!(
            waproto::whatsapp::message::CancelPaymentRequestMessage {
                key: Some(nx!(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.request_message_id),
                    from_me: Some(false),
                    ..Default::default()
                }))
                .into(),
            }
        ),),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/decline-payment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = DeclinePaymentRequestRequest,
    responses(
        (status = 200, description = "Payment request declined", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn decline_payment_request(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<DeclinePaymentRequestRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "decline-payment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_decline_payment_request(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `decline-payment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_decline_payment_request(
    state: &AppState,
    session_id: &str,
    request: DeclinePaymentRequestRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let message = nx!(waproto::whatsapp::Message {
        decline_payment_request_message: MessageField::some(nx!(
            waproto::whatsapp::message::DeclinePaymentRequestMessage {
                key: Some(nx!(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.request_message_id),
                    from_me: Some(false),
                    ..Default::default()
                }))
                .into(),
            }
        ),),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}
