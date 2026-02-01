//! TURN relay credential handlers

use axum::{
    extract::State,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::{error::{ApiError, ApiResult}, turn_credentials, AppState};

#[derive(Debug, Deserialize)]
pub struct CredentialsRequest {
    /// User/AI ID requesting credentials
    pub user_id: String,
    /// Requested TTL in seconds (default: 86400 = 24 hours)
    #[serde(default = "default_ttl")]
    pub ttl: u64,
}

fn default_ttl() -> u64 {
    86400 // 24 hours
}

/// Get TURN credentials for relay access
pub async fn get_credentials(
    State(state): State<AppState>,
    Json(req): Json<CredentialsRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.user_id.is_empty() {
        return Err(ApiError::bad_request("user_id is required"));
    }

    // Get TURN secret
    let secret = state.turn_secret.as_ref().ok_or_else(|| {
        ApiError::service_unavailable("TURN credentials not configured")
    })?;

    // Limit TTL to max 7 days
    let ttl = req.ttl.min(604800);

    // Generate credentials
    let creds = turn_credentials::generate_credentials(
        &req.user_id,
        secret,
        state.turn_servers.clone(),
        ttl,
    );

    Ok(Json(creds))
}
