//! Chat-level actions synced to every linked device through app state:
//! archive, pin, mute, read state, clear, delete, disappearing messages,
//! starring and delete-for-me. Plus the chat list and starred messages as
//! the session's chat store knows them.

pub mod handlers;
pub mod models;
pub mod routes;
