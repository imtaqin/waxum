use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request to mint a session-scoped bearer token.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MintTokenRequest {
    /// Human-readable label for this token (shown in `GET /tokens`, not
    /// used for anything security-relevant).
    #[schema(example = "customer-mobile-app")]
    pub name: Option<String>,
    /// Session IDs this token may access. Each must already exist. At least
    /// one of `session_ids` / `session_prefixes` must be non-empty -- a
    /// token scoped to nothing can't reach any `/sessions/{id}/*` route.
    #[schema(example = json!(["my-session"]))]
    #[serde(default)]
    pub session_ids: Vec<String>,
    /// Session-id prefixes this token may use, e.g. `["rq-mkt-"]`. The
    /// token can then reach every session whose id starts with one of
    /// them, create new sessions with such an id (`POST /sessions`), delete
    /// them, and list them (`GET /sessions` returns only sessions in
    /// scope). At least 3 characters of `A-Z a-z 0-9 - _`, so a prefix can
    /// never widen a token to every session.
    #[schema(example = json!(["rq-mkt-"]))]
    #[serde(default)]
    pub session_prefixes: Vec<String>,
    /// How long the token stays valid. Defaults to 720 (30 days) when omitted.
    #[schema(example = 720)]
    pub expires_in_hours: Option<i64>,
}

/// A freshly minted token -- the only time the raw bearer value is ever
/// returned. `GET /tokens` only shows metadata, never the token itself.
#[derive(Debug, Serialize, ToSchema)]
pub struct MintTokenResponse {
    /// Token id (the JWT's `jti`), used to revoke it later via
    /// `POST /tokens/{id}/revoke`.
    pub id: String,
    /// Bearer token. Store it now -- it can't be retrieved again.
    pub token: String,
    pub name: Option<String>,
    pub session_ids: Vec<String>,
    pub session_prefixes: Vec<String>,
    /// Unix timestamp the token stops validating.
    pub expires_at: i64,
}

/// Metadata for a minted token -- never includes the bearer value itself.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenSummary {
    pub id: String,
    pub name: Option<String>,
    pub session_ids: Vec<String>,
    pub session_prefixes: Vec<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TokenListResponse {
    pub tokens: Vec<TokenSummary>,
    pub count: usize,
}
