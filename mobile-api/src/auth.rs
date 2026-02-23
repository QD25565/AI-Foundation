//! Bearer token authentication middleware for axum.

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;
use crate::AppState;

/// Extractor that validates the Bearer token.
/// Carries both the resolved `h_id` and the raw `token` string so handlers
/// that need to revoke (e.g. unpair) don't have to re-parse the header.
pub struct AuthUser {
    pub h_id: String,
    pub token: String,
}

#[async_trait]
impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let token = if let Some(tok) = auth_header.strip_prefix("Bearer ") {
            tok.trim().to_string()
        } else {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "ok": false, "error": "Missing or invalid Authorization header" })),
            )
                .into_response());
        };

        let h_id = state
            .pairing
            .lookup_token(&token)
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "ok": false, "error": "Invalid or expired token" })),
                )
                    .into_response()
            })?;

        Ok(AuthUser { h_id, token })
    }
}
