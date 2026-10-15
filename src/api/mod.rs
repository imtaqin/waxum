//! HTTP surface, grouped by channel and then by feature.

/// Axum's `Bytes`-based extractors (`Multipart` fields among them) cap
/// bodies at 2MB by default -- fine for JSON, too small for real media or a
/// session storage archive. These routes get an explicit, deliberate raise
/// instead of silently inheriting that default or being left unbounded.
pub const MAX_MEDIA_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
pub const MAX_SESSION_IMPORT_BYTES: usize = 512 * 1024 * 1024;

pub mod common;
pub mod openapi;
pub mod router;
pub mod shared;

pub mod cloud;

pub mod web;
