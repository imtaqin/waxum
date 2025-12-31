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

/// JWT Claims structure
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user identifier)
    pub sub: String,
    /// Role (e.g., "superadmin")
    pub role: String,
    /// Expiration time (Unix timestamp)
    pub exp: usize,
    /// Issued at (Unix timestamp)
    pub iat: usize,
}

/// JWT Authentication configuration
#[derive(Clone)]
pub struct JwtAuth {
    secret: String,
}

impl JwtAuth {
    /// Create new JWT auth with secret from environment
    pub fn new() -> Self {
        let secret = std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "your-super-secret-jwt-key-change-in-production".to_string());
        Self { secret }
    }

    /// Create new JWT auth with custom secret
    #[allow(dead_code)]
    pub fn with_secret(secret: String) -> Self {
        Self { secret }
    }

    /// Generate a new JWT token for superadmin
    pub fn generate_token(&self, subject: &str, role: &str, expires_in_hours: i64) -> Result<String, jsonwebtoken::errors::Error> {
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

    /// Validate a JWT token and return claims
    pub fn validate_token(&self, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
        )?;
        Ok(token_data.claims)
    }

    /// Check if claims have superadmin role
    pub fn is_superadmin(claims: &Claims) -> bool {
        claims.role == "superadmin"
    }
}

/// Middleware function to authenticate JWT tokens
pub async fn jwt_auth_middleware(
    request: Request<Body>,
    next: Next,
) -> Response {
    // Skip auth for health check
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }

    // Skip auth for swagger UI and OpenAPI docs
    if request.uri().path().starts_with("/swagger-ui")
        || request.uri().path().starts_with("/api-docs")
    {
        return next.run(request).await;
    }

    // Get JWT secret from environment
    let jwt_auth = JwtAuth::new();

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let token = match auth_header {
        Some(header) if header.starts_with("Bearer ") => &header[7..],
        _ => {
            return unauthorized_response("Missing or invalid Authorization header. Use: Bearer <token>");
        }
    };

    // Validate token
    match jwt_auth.validate_token(token) {
        Ok(claims) => {
            // Check if user has superadmin role
            if !JwtAuth::is_superadmin(&claims) {
                return forbidden_response("Superadmin role required");
            }
            // Token is valid and user is superadmin, proceed
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

/// Helper to generate a superadmin token (for initial setup)
pub fn generate_superadmin_token() -> String {
    let jwt_auth = JwtAuth::new();
    jwt_auth
        .generate_token("superadmin", "superadmin", 24 * 365) // 1 year expiry
        .expect("Failed to generate token")
}
