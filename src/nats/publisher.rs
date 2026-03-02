use async_nats::jetstream;
use bytes::Bytes;

/// Publish a WhatsApp event to NATS JetStream.
/// Subject format: `wa.events.{session_id}.{event_type}`
pub async fn publish_event(
    jetstream: &jetstream::Context,
    session_id: &str,
    event_type: &str,
    payload: &str,
) {
    let subject = format!("wa.events.{}.{}", session_id, event_type);

    match jetstream
        .publish(subject.clone(), Bytes::from(payload.to_string()))
        .await
    {
        Ok(ack_future) => {
            // Await the ack to ensure JetStream persisted the message
            if let Err(e) = ack_future.await {
                tracing::warn!("NATS JetStream ack failed for {}: {}", subject, e);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to publish event to NATS {}: {}", subject, e);
        }
    }
}
