//! Authentication Handlers
//!
//! Register AIs, generate tokens, refresh tokens.

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use crate::{
    auth::{password, Claims, JwtManager, TokenPair},
    db::queries,
    error::{ApiError, ApiResult},
    AppState,
};

/// Register a new AI
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub ai_id: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub ai_id: String,
    pub message: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<impl IntoResponse> {
    // Validate ai_id format
    if req.ai_id.len() < 3 || req.ai_id.len() > 64 {
        return Err(ApiError::bad_request("AI ID must be 3-64 characters"));
    }

    if !req.ai_id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(ApiError::bad_request(
            "AI ID must contain only alphanumeric characters, hyphens, and underscores",
        ));
    }

    // Validate password
    if req.password.len() < 8 {
        return Err(ApiError::bad_request("Password must be at least 8 characters"));
    }

    // Hash password
    let password_hash = password::hash(&req.password)
        .map_err(|e| ApiError::internal(format!("Password hashing failed: {}", e)))?;

    // Register in database
    let conn = state.db.get().await?;
    queries::register_ai(
        &conn,
        &req.ai_id,
        req.display_name.as_deref(),
        &password_hash,
    )
    .await?;

    Ok(Json(RegisterResponse {
        ai_id: req.ai_id,
        message: "Registration successful. Use /v1/auth/token to get access token.".to_string(),
    }))
}

/// Get access token (login)
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub ai_id: String,
    pub password: String,
}

pub async fn get_token(
    State(state): State<AppState>,
    Json(req): Json<TokenRequest>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Get stored password hash
    let hash = queries::get_ai_password_hash(&conn, &req.ai_id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("Invalid credentials"))?;

    // Verify password
    let valid = password::verify(&req.password, &hash)
        .map_err(|_| ApiError::internal("Password verification failed"))?;

    if !valid {
        return Err(ApiError::unauthorized("Invalid credentials"));
    }

    // Generate tokens (default to free tier)
    let jwt_manager = JwtManager::new(&state.config.jwt_secret);
    let tokens = jwt_manager
        .generate_token_pair(&req.ai_id, "free")
        .map_err(|e| ApiError::internal(format!("Token generation failed: {}", e)))?;

    Ok(Json(tokens))
}

/// Refresh access token
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

pub async fn refresh_token(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> ApiResult<impl IntoResponse> {
    let jwt_manager = JwtManager::new(&state.config.jwt_secret);

    // Validate refresh token
    let claims = jwt_manager.validate(&req.refresh_token)?;

    if !claims.is_refresh() {
        return Err(ApiError::bad_request("Expected refresh token, got access token"));
    }

    // Generate new token pair
    let tokens = jwt_manager
        .generate_token_pair(&claims.sub, &claims.tier)
        .map_err(|e| ApiError::internal(format!("Token generation failed: {}", e)))?;

    Ok(Json(tokens))
}

/// Generate API key
#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    pub api_key: String,
    pub message: String,
}

pub async fn generate_api_key(
    State(state): State<AppState>,
    auth: crate::auth::AuthenticatedAi,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let (_, key) = queries::create_api_key(&conn, &auth.ai_id, &auth.tier).await?;

    Ok(Json(ApiKeyResponse {
        api_key: key,
        message: "Store this key securely. It cannot be retrieved again.".to_string(),
    }))
}
