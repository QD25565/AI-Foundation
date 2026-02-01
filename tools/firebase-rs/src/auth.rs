//! Firebase Authentication via Service Account
//!
//! Uses service account JSON to generate OAuth2 access tokens
//! for Firebase REST API authentication.

use crate::error::{FirebaseError, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Service account credentials from JSON file
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAccount {
    #[serde(rename = "type")]
    pub account_type: String,
    pub project_id: String,
    pub private_key_id: String,
    pub private_key: String,
    pub client_email: String,
    pub client_id: String,
    pub auth_uri: String,
    pub token_uri: String,
}

impl ServiceAccount {
    /// Load service account from JSON file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(FirebaseError::ServiceAccountNotFound(
                path.display().to_string(),
            ));
        }

        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).map_err(|e| {
            FirebaseError::InvalidServiceAccount(format!("Failed to parse service account: {}", e))
        })
    }

    /// Load from environment variable path
    pub fn from_env() -> Result<Self> {
        let path = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").map_err(|_| {
            FirebaseError::ServiceAccountNotFound(
                "GOOGLE_APPLICATION_CREDENTIALS not set".to_string(),
            )
        })?;
        Self::from_file(&path)
    }
}

/// JWT claims for Google OAuth2
#[derive(Debug, Serialize)]
struct JwtClaims {
    iss: String,     // Issuer (service account email)
    sub: String,     // Subject (same as issuer for service accounts)
    aud: String,     // Audience (token URI)
    iat: u64,        // Issued at
    exp: u64,        // Expiration
    scope: String,   // OAuth scopes
}

/// OAuth2 token response - Google may return access_token, id_token, or both
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
}

/// Cached access token
struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// Firebase authentication manager
pub struct FirebaseAuth {
    service_account: ServiceAccount,
    http_client: reqwest::Client,
    cached_token: Arc<RwLock<Option<CachedToken>>>,
}

impl FirebaseAuth {
    /// Create new auth manager from service account
    pub fn new(service_account: ServiceAccount) -> Self {
        Self {
            service_account,
            http_client: reqwest::Client::new(),
            cached_token: Arc::new(RwLock::new(None)),
        }
    }

    /// Create from service account file path
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let sa = ServiceAccount::from_file(path)?;
        Ok(Self::new(sa))
    }

    /// Create from environment variable
    pub fn from_env() -> Result<Self> {
        let sa = ServiceAccount::from_env()?;
        Ok(Self::new(sa))
    }

    /// Get the project ID
    pub fn project_id(&self) -> &str {
        &self.service_account.project_id
    }

    /// Get access token, using cache if valid
    pub async fn get_access_token(&self) -> Result<String> {
        // Check cache first
        {
            let cache = self.cached_token.read().await;
            if let Some(ref cached) = *cache {
                // Use token if it has at least 60 seconds remaining
                if cached.expires_at > Instant::now() + Duration::from_secs(60) {
                    return Ok(cached.token.clone());
                }
            }
        }

        // Generate new token
        let token = self.generate_access_token().await?;

        Ok(token)
    }

    /// Generate new access token via OAuth2
    async fn generate_access_token(&self) -> Result<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Create JWT claims
        let claims = JwtClaims {
            iss: self.service_account.client_email.clone(),
            sub: self.service_account.client_email.clone(),
            aud: self.service_account.token_uri.clone(),
            iat: now,
            exp: now + 3600, // 1 hour
            // cloud-platform scope covers all Firebase/GCP services
            scope: "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/playdeveloperreporting".to_string(),
        };

        // Create JWT header
        let header = Header {
            alg: Algorithm::RS256,
            kid: Some(self.service_account.private_key_id.clone()),
            ..Default::default()
        };

        // Sign JWT with private key
        let key = EncodingKey::from_rsa_pem(self.service_account.private_key.as_bytes())?;
        let jwt = encode(&header, &claims, &key)?;

        // Exchange JWT for access token
        let response = self
            .http_client
            .post(&self.service_account.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(FirebaseError::from_response(status.as_u16(), &body));
        }

        let token_response: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            FirebaseError::TokenError(format!("Failed to parse token response: {}. Body: {}", e, &body[..body.len().min(500)]))
        })?;

        // Prefer access_token, fall back to id_token for Firebase APIs
        let token = token_response.access_token
            .or(token_response.id_token)
            .ok_or_else(|| FirebaseError::TokenError("No access_token or id_token in response".to_string()))?;

        let expires_in = token_response.expires_in.unwrap_or(3600);

        // Cache the token
        {
            let mut cache = self.cached_token.write().await;
            *cache = Some(CachedToken {
                token: token.clone(),
                expires_at: Instant::now() + Duration::from_secs(expires_in),
            });
        }

        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_account_missing_file() {
        let result = ServiceAccount::from_file("/nonexistent/path.json");
        assert!(matches!(result, Err(FirebaseError::ServiceAccountNotFound(_))));
    }
}
