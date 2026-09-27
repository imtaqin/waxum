//! Bearer-token authentication middleware.
//!
//! Every request through `/api/v1/*` passes through [`jwt_auth_middleware`].
//! It accepts either:
//!
//! - the plain-string `SUPERADMIN_TOKEN` (env), or
//! - a JWT signed with `JWT_SECRET` whose `role` claim is `superadmin`.
//!
//! A token minted via `POST /api/v1/tokens` carries a `jti`; that gets
//! looked up in the `tokens`/`token_sessions` tables (see
//! [`crate::db::tokens`]) on every request to enforce revocation and
//! session scope. Tokens without a `jti` -- `SUPERADMIN_TOKEN` and the
//! unscoped JWT printed at boot -- skip that lookup entirely and reach
//! every session, unchanged from before this existed.
//!
//! `/health` and `/metrics` are always allowed through so probes and
//! scrapers don't need credentials. Any `.../cloud/webhook` path is also
//! exempt -- Meta calls it directly and only ever presents its own
//! `X-Hub-Signature-256`, verified per-session inside the handler against
//! that session's own `cloud_app_secret`, never a waxum bearer token.
//! `.../cloud/flow-endpoint/exchange` (the WhatsApp Flows Data Exchange
//! endpoint) is exempt for the same reason; see
//! [`crate::handlers::cloud_flows`] for its signature + RSA/AES checks.
//! `/swagger-ui` and `/api-docs` go
//! through the same check as everything else — either header carries the
//! plain `SUPERADMIN_TOKEN` (or a superadmin JWT), or the browser already
//! holds the `waxum_console` cookie from a console login, so a signed-in
//! operator lands straight on the docs with no separate prompt.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use once_cell::sync::Lazy;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::AppState;

/// Process-local fallback signing secret, used only when `JWT_SECRET` is
/// unset. Generated once per process from OS entropy rather than a fixed
/// string baked into the binary — a hardcoded fallback here would let anyone
/// who has read this source forge a superadmin token against any instance
/// that forgot to set `JWT_SECRET`. The tradeoff: tokens signed against this
/// fallback stop validating on restart, which is the point — it bounds how
/// long a leaked token (e.g. from the boot-banner print) stays useful.
static FALLBACK_JWT_SECRET: Lazy<String> = Lazy::new(|| {
    tracing::warn!(
        "JWT_SECRET is not set; using a random secret generated for this \
         process only. Tokens will stop validating after a restart. Set \
         JWT_SECRET to a strong, stable value before deploying to production."
    );
    let bytes: [u8; 32] = rand::thread_rng().gen();
    hex::encode(bytes)
});

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,

    pub role: String,

    pub exp: usize,

    pub iat: usize,

    /// Identifies this token's row in the `tokens` table (see
    /// [`crate::db::tokens`]), checked on every request in
    /// [`jwt_auth_middleware`] for revocation and session-scope
    /// enforcement. `None` (the default for tokens issued before this field
    /// existed, and for the operator-wide `SUPERADMIN_TOKEN`/boot token) is
    /// unrestricted and skips the DB lookup entirely -- unchanged,
    /// pre-existing behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
}

#[derive(Clone)]
pub struct JwtAuth {
    secret: String,
}

impl Default for JwtAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl JwtAuth {
    pub fn new() -> Self {
        let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| FALLBACK_JWT_SECRET.clone());
        Self { secret }
    }

    #[allow(dead_code)]
    pub fn with_secret(secret: String) -> Self {
        Self { secret }
    }

    pub fn generate_token(
        &self,
        subject: &str,
        role: &str,
        expires_in_hours: i64,
        jti: Option<&str>,
    ) -> Result<String, jsonwebtoken::errors::Error> {
        let now = chrono::Utc::now();
        let exp = (now + chrono::Duration::hours(expires_in_hours)).timestamp() as usize;
        let iat = now.timestamp() as usize;

        let claims = Claims {
            sub: subject.to_string(),
            role: role.to_string(),
            exp,
            iat,
            jti: jti.map(|s| s.to_string()),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
    }

    pub fn validate_token(&self, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
        )?;
        Ok(token_data.claims)
    }

    pub fn is_superadmin(claims: &Claims) -> bool {
        claims.role == "superadmin"
    }
}

/// Best-effort caller IP for auth-failure logging. `main.rs` serves via
/// `into_make_service_with_connect_info::<SocketAddr>()`, so this is
/// present on every real request; falls back gracefully (e.g. in tests that
/// build the router without connect-info) rather than panicking.
fn peer_ip(request: &Request<Body>) -> String {
    request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

pub async fn jwt_auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if matches!(path, "/health" | "/livez" | "/readyz" | "/metrics") {
        return next.run(request).await;
    }

    if path.ends_with("/cloud/webhook") || path.ends_with("/cloud/flow-endpoint/exchange") {
        return next.run(request).await;
    }

    if crate::console::is_console_path(path) {
        return next.run(request).await;
    }

    let jwt_auth = JwtAuth::new();

    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let cookie_token = request
        .headers()
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|raw| {
            for part in raw.split(';') {
                let p = part.trim();
                if let Some(rest) = p.strip_prefix("waxum_console=") {
                    return Some(rest.to_string());
                }
            }
            None
        });

    let peer = peer_ip(&request);

    let token: String = match auth_header {
        Some(header) if header.starts_with("Bearer ") => header[7..].to_string(),
        _ => match cookie_token {
            Some(t) => t,
            None => {
                tracing::warn!(peer = %peer, path = %path, "auth: missing bearer token/cookie");
                return unauthorized_response(
                    "Missing or invalid Authorization header. Use: Bearer <token>",
                );
            }
        },
    };
    let token = token.as_str();

    if let Ok(superadmin_token) = std::env::var("SUPERADMIN_TOKEN") {
        if !superadmin_token.is_empty() && constant_time_eq(token, &superadmin_token) {
            return next.run(request).await;
        }
    }

    match jwt_auth.validate_token(token) {
        Ok(claims) => {
            if !JwtAuth::is_superadmin(&claims) {
                tracing::warn!(peer = %peer, path = %path, role = %claims.role, "auth: non-superadmin role rejected");
                return forbidden_response("Superadmin role required");
            }

            if let Some(jti) = claims.jti.as_deref() {
                let store = crate::db::tokens::TokenStore::new(state.session_manager().pool());
                let status = match store.status(jti).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(peer = %peer, path = %path, error = %e, "auth: token status lookup failed");
                        return unauthorized_response("Token status lookup failed");
                    }
                };
                let Some(status) = status else {
                    tracing::warn!(peer = %peer, path = %path, jti = %jti, "auth: token has no matching record (revoked/deleted?)");
                    return forbidden_response("Token is no longer valid");
                };
                if status.revoked {
                    tracing::warn!(peer = %peer, path = %path, jti = %jti, "auth: revoked token used");
                    return forbidden_response("Token has been revoked");
                }
                match session_id_from_path(path) {
                    Some(requested) if status.session_ids.iter().any(|s| s == requested) => {}
                    _ => {
                        tracing::warn!(
                            peer = %peer, path = %path, jti = %jti,
                            "auth: token attempted access outside its bound sessions"
                        );
                        return forbidden_response("Token is not authorized for this session");
                    }
                }
            }

            next.run(request).await
        }
        Err(e) => {
            tracing::warn!(peer = %peer, path = %path, error = %e, "auth: token validation failed");
            let message = match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => "Token has expired",
                jsonwebtoken::errors::ErrorKind::InvalidToken => "Invalid token format",
                jsonwebtoken::errors::ErrorKind::InvalidSignature => "Invalid token signature",
                _ => "Token validation failed",
            };
            unauthorized_response(message)
        }
    }
}

/// Pulls the `{session_id}` segment out of a `/api/v1/sessions/{session_id}/...`
/// request path, or `None` for anything else -- including the fleet-wide
/// operations nested under the same `/sessions` prefix (`/`, `/purge`,
/// `/disconnect-all`, `/reconnect-all`, `/search`), which a session-scoped
/// token must not be able to reach since they aren't bounded to one session.
fn session_id_from_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/v1/sessions/")?;
    let id = rest.split('/').next()?;
    if id.is_empty() || matches!(id, "purge" | "disconnect-all" | "reconnect-all" | "search") {
        return None;
    }
    Some(id)
}

/// Constant-time string comparison for secret/token checks. A plain `==`
/// short-circuits on the first mismatched byte, which turns a bearer-token
/// check into a timing oracle for brute-forcing a long-lived secret one byte
/// at a time; this walks the full length regardless of where it first
/// differs.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn unauthorized_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "Unauthorized",
            "message": message
        })),
    )
        .into_response()
}

fn forbidden_response(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "Forbidden",
            "message": message
        })),
    )
        .into_response()
}

pub fn get_superadmin_token() -> (String, bool) {
    if let Ok(token) = std::env::var("SUPERADMIN_TOKEN") {
        if !token.is_empty() {
            return (token, true);
        }
    }

    let jwt_auth = JwtAuth::new();
    let token = jwt_auth
        .generate_token("superadmin", "superadmin", 24 * 365, None)
        .expect("Failed to generate token");
    (token, false)
}

#[allow(dead_code)]
pub fn generate_superadmin_token() -> String {
    let jwt_auth = JwtAuth::new();
    jwt_auth
        .generate_token("superadmin", "superadmin", 24 * 365, None)
        .expect("Failed to generate token")
}
