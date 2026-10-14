//! Routes for messages.

use super::handlers;
use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/messages/text", post(handlers::send_text))
        .route("/{session_id}/messages/image", post(handlers::send_image))
        .route("/{session_id}/messages/video", post(handlers::send_video))
        .route("/{session_id}/messages/audio", post(handlers::send_audio))
        .route(
            "/{session_id}/messages/document",
            post(handlers::send_document),
        )
        .route(
            "/{session_id}/messages/sticker",
            post(handlers::send_sticker),
        )
        .route(
            "/{session_id}/messages/location",
            post(handlers::send_location),
        )
        .route(
            "/{session_id}/messages/contact",
            post(handlers::send_contact),
        )
        .route("/{session_id}/messages/edit", post(handlers::edit_message))
        .route(
            "/{session_id}/messages/react",
            post(handlers::send_reaction),
        )
        .route(
            "/{session_id}/messages/revoke",
            post(handlers::revoke_message),
        )
        .route("/{session_id}/messages/read", post(handlers::mark_as_read))
        .route("/{session_id}/messages/poll", post(handlers::send_poll))
        .route(
            "/{session_id}/messages/buttons",
            post(handlers::send_buttons),
        )
        .route("/{session_id}/messages/list", post(handlers::send_list))
        .route(
            "/{session_id}/messages/interactive",
            post(handlers::send_interactive),
        )
        .route(
            "/{session_id}/messages/cta-url",
            post(handlers::send_cta_url),
        )
        .route(
            "/{session_id}/messages/quick-reply",
            post(handlers::send_quick_reply),
        )
        .route(
            "/{session_id}/messages/newsletter-admin-invite",
            post(handlers::send_newsletter_admin_invite),
        )
        .route(
            "/{session_id}/messages/newsletter-follower-invite",
            post(handlers::send_newsletter_follower_invite),
        )
        .route("/{session_id}/messages/order", post(handlers::send_order))
        .route(
            "/{session_id}/messages/invoice",
            post(handlers::send_invoice),
        )
        .route(
            "/{session_id}/messages/payment-invite",
            post(handlers::send_payment_invite),
        )
        .route(
            "/{session_id}/messages/pin",
            post(handlers::send_pin_message),
        )
        .route(
            "/{session_id}/messages/forward",
            post(handlers::forward_message),
        )
        .route(
            "/{session_id}/messages/poll-update",
            post(handlers::send_poll_update),
        )
        .route(
            "/{session_id}/messages/buttons-response",
            post(handlers::send_buttons_response),
        )
        .route(
            "/{session_id}/messages/list-response",
            post(handlers::send_list_response),
        )
        .route(
            "/{session_id}/messages/interactive-response",
            post(handlers::send_interactive_response),
        )
        .route(
            "/{session_id}/messages/highly-structured",
            post(handlers::send_highly_structured),
        )
        .route(
            "/{session_id}/messages/template-button-reply",
            post(handlers::send_template_button_reply),
        )
        .route(
            "/{session_id}/messages/comment",
            post(handlers::send_comment),
        )
        .route(
            "/{session_id}/messages/scheduled-call",
            post(handlers::send_scheduled_call),
        )
        .route(
            "/{session_id}/messages/scheduled-call-edit",
            post(handlers::send_scheduled_call_edit),
        )
        .route(
            "/{session_id}/messages/send-payment",
            post(handlers::send_payment),
        )
        .route(
            "/{session_id}/messages/request-payment",
            post(handlers::request_payment),
        )
        .route(
            "/{session_id}/messages/cancel-payment",
            post(handlers::cancel_payment_request),
        )
        .route(
            "/{session_id}/messages/decline-payment",
            post(handlers::decline_payment_request),
        )
        .route(
            "/{session_id}/messages/newsletter-forward",
            post(handlers::send_newsletter_forward),
        )
}
