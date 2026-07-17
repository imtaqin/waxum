//! Bearer-token authentication middleware.
//!
//! Every request through `/api/v1/*` passes through [`jwt_auth_middleware`].
//! It accepts either:
//!
//! - the plain-string `SUPERADMIN_TOKEN` (env), or
//! - a JWT signed with `JWT_SECRET` whose `role` claim is `superadmin`.
//!
//! `/health` and `/metrics` are always allowed through so probes and
//! scrapers don't need credentials. `/swagger-ui` and `/api-docs` are also
//! exempt so the interactive docs can render without auth.

use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,

    pub role: String,

    pub exp: usize,

    pub iat: usize,
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
        let secret = std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "your-super-secret-jwt-key-change-in-production".to_string());
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
    ) -> Result<String, jsonwebtoken::errors::Error> {
        let now = chrono::Utc::now();
        let exp = (now + chrono::Duration::hours(expires_in_hours)).timestamp() as usize;
        let iat = now.timestamp() as usize;

        let claims = Claims {
            sub: subject.to_string(),
            role: role.to_string(),
            exp,
            iat,
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

pub async fn jwt_auth_middleware(request: Request<Body>, next: Next) -> Response {
    let path = request.uri().path();
    if matches!(path, "/health" | "/livez" | "/readyz" | "/metrics") {
        return next.run(request).await;
    }

    if request.uri().path().starts_with("/swagger-ui")
        || request.uri().path().starts_with("/api-docs")
    {
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

    let token: String = match auth_header {
        Some(header) if header.starts_with("Bearer ") => header[7..].to_string(),
        _ => match cookie_token {
            Some(t) => t,
            None => {
                return unauthorized_response(
                    "Missing or invalid Authorization header. Use: Bearer <token>",
                );
            }
        },
    };
    let token = token.as_str();

    if let Ok(superadmin_token) = std::env::var("SUPERADMIN_TOKEN") {
        if !superadmin_token.is_empty() && token == superadmin_token {
            return next.run(request).await;
        }
    }

    match jwt_auth.validate_token(token) {
        Ok(claims) => {
            if !JwtAuth::is_superadmin(&claims) {
                return forbidden_response("Superadmin role required");
            }

            next.run(request).await
        }
        Err(e) => {
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
        .generate_token("superadmin", "superadmin", 24 * 365)
        .expect("Failed to generate token");
    (token, false)
}

#[allow(dead_code)]
pub fn generate_superadmin_token() -> String {
    let jwt_auth = JwtAuth::new();
    jwt_auth
        .generate_token("superadmin", "superadmin", 24 * 365)
        .expect("Failed to generate token")
}
