use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::models::sessions::SessionStatus;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PurgeQuery {
    #[serde(default = "default_purge_filter")]
    pub filter: String,
    #[serde(default = "default_purge_days")]
    pub days: i64,
    #[serde(default)]
    pub dry_run: bool,
}

fn default_purge_filter() -> String {
    "inactive".to_string()
}
fn default_purge_days() -> i64 {
    30
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PurgeResponse {
    pub filter: String,
    pub days: i64,
    pub dry_run: bool,
    pub purged: Vec<String>,
    pub kept: usize,
    pub total_before: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DisconnectAllResponse {
    pub disconnected: Vec<String>,
    pub skipped: Vec<String>,
    pub total: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReconnectAllResponse {
    pub scheduled: Vec<String>,
    pub skipped: Vec<String>,
    pub total: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchHit {
    pub id: String,
    pub name: Option<String>,
    pub phone_number: Option<String>,
    pub push_name: Option<String>,
    pub status: SessionStatus,
    pub is_logged_in: bool,
    pub match_on: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    pub q: String,
    pub total: usize,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FleetStats {
    pub session_total: usize,
    pub session_connected: usize,
    pub session_connecting: usize,
    pub session_disconnected: usize,
    pub session_logged_out: usize,
    pub webhook_total: usize,
    pub webhook_circuits_open: usize,
    pub event_rate_per_min: u32,
    pub uptime_seconds: u64,
    pub version: String,
    pub storage_path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReenableCircuitsResponse {
    pub reenabled: Vec<String>,
    pub total: usize,
}
