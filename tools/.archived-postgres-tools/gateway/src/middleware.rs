//! Authentication Middleware
//!
//! Extracts and validates authentication from requests.
//! Supports both JWT Bearer tokens and API keys.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use crate::{
    auth::{extract_api_key, extract_bearer_token, AuthMethod, AuthenticatedAi, JwtManager},
    db::queries,
    error::ApiError,
    AppState,
};

/// Authentication middleware
///
/// Checks for authentication in this order:
/// 1. Authorization: Bearer <jwt>
/// 2. X-API-Key header
/// 3. api_key query parameter
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let headers = request.headers();

    // Try JWT Bearer token first
    if let Some(auth_header) = headers.get("Authorization").and_then(|v| v.to_str().ok()) {
        if let Some(token) = extract_bearer_token(auth_header) {
            let jwt_manager = JwtManager::new(&state.config.jwt_secret);
            let claims = jwt_manager.validate(token)?;

            if !claims.is_access() {
                return Err(ApiError::unauthorized("Refresh tokens cannot be used for API access"));
            }

            let auth = AuthenticatedAi {
                ai_id: claims.sub,
                tier: claims.tier,
                auth_method: AuthMethod::Jwt,
            };

            request.extensions_mut().insert(auth);
            return Ok(next.run(request).await);
        }
    }

    // Try API key
    let api_key = extract_api_key(
        headers,
        request.uri().query(),
    );

    if let Some(key) = api_key {
        let conn = state.db.get().await?;

        if let Some(record) = queries::get_api_key(&conn, &key).await? {
            if record.revoked {
                return Err(ApiError::unauthorized("API key has been revoked"));
            }

            // Update last used timestamp (fire and forget)
            let key_clone = key.clone();
            let db = state.db.clone();
            tokio::spawn(async move {
                if let Ok(conn) = db.get().await {
                    let _ = queries::update_api_key_last_used(&conn, &key_clone).await;
                }
            });

            let auth = AuthenticatedAi {
                ai_id: record.ai_id,
                tier: record.tier,
                auth_method: AuthMethod::ApiKey,
            };

            request.extensions_mut().insert(auth);
            return Ok(next.run(request).await);
        }
    }

    Err(ApiError::unauthorized("Missing or invalid authentication"))
}

/// Optional authentication middleware
///
/// Like auth_middleware but allows unauthenticated requests through.
/// Sets AuthenticatedAi extension only if valid auth is present.
pub async fn optional_auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();

    // Try JWT Bearer token first
    if let Some(auth_header) = headers.get("Authorization").and_then(|v| v.to_str().ok()) {
        if let Some(token) = extract_bearer_token(auth_header) {
            let jwt_manager = JwtManager::new(&state.config.jwt_secret);
            if let Ok(claims) = jwt_manager.validate(token) {
                if claims.is_access() {
                    let auth = AuthenticatedAi {
                        ai_id: claims.sub,
                        tier: claims.tier,
                        auth_method: AuthMethod::Jwt,
                    };
                    request.extensions_mut().insert(auth);
                }
            }
        }
    }

    // Try API key if no JWT
    if request.extensions().get::<AuthenticatedAi>().is_none() {
        let api_key = extract_api_key(
            request.headers(),
            request.uri().query(),
        );

        if let Some(key) = api_key {
            if let Ok(conn) = state.db.get().await {
                if let Ok(Some(record)) = queries::get_api_key(&conn, &key).await {
                    if !record.revoked {
                        let auth = AuthenticatedAi {
                            ai_id: record.ai_id,
                            tier: record.tier,
                            auth_method: AuthMethod::ApiKey,
                        };
                        request.extensions_mut().insert(auth);
                    }
                }
            }
        }
    }

    next.run(request).await
}

/// Rate limiting middleware
pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Get authenticated AI if present
    let (ai_id, tier) = if let Some(auth) = request.extensions().get::<AuthenticatedAi>() {
        (auth.ai_id.clone(), auth.tier.clone())
    } else {
        // Anonymous requests get free tier limits
        let ip = request
            .headers()
            .get("X-Forwarded-For")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string();
        (ip, "free".to_string())
    };

    // Check rate limit
    if !state.rate_limiter.check(&ai_id, &tier).await {
        return Err(ApiError::rate_limited());
    }

    // Add rate limit headers to response
    let mut response = next.run(request).await;

    let limit = state.rate_limiter.get_limit(&tier);
    let remaining = state.rate_limiter.remaining(&ai_id, &tier).await;

    let headers = response.headers_mut();
    headers.insert("X-RateLimit-Limit", limit.to_string().parse().unwrap());
    headers.insert("X-RateLimit-Remaining", remaining.to_string().parse().unwrap());
    headers.insert("X-RateLimit-Reset", "60".parse().unwrap());

    Ok(response)
}

/// Extract authenticated AI from request extensions
pub fn get_authenticated_ai(request: &Request) -> Option<&AuthenticatedAi> {
    request.extensions().get::<AuthenticatedAi>()
}
