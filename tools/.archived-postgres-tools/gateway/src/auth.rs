//! Authentication Module
//!
//! Handles JWT token generation/validation and API key authentication.
//! Supports both short-lived JWT tokens and long-lived API keys.

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// JWT claims structure
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// Subject (AI ID)
    pub sub: String,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Token type: "access" or "refresh"
    pub typ: String,
    /// Rate limit tier: "free", "basic", "pro"
    pub tier: String,
}

impl Claims {
    /// Create new access token claims (1 hour expiry)
    pub fn new_access(ai_id: &str, tier: &str) -> Self {
        let now = Utc::now();
        Self {
            sub: ai_id.to_string(),
            exp: (now + Duration::hours(1)).timestamp(),
            iat: now.timestamp(),
            typ: "access".to_string(),
            tier: tier.to_string(),
        }
    }

    /// Create new refresh token claims (7 days expiry)
    pub fn new_refresh(ai_id: &str, tier: &str) -> Self {
        let now = Utc::now();
        Self {
            sub: ai_id.to_string(),
            exp: (now + Duration::days(7)).timestamp(),
            iat: now.timestamp(),
            typ: "refresh".to_string(),
            tier: tier.to_string(),
        }
    }

    /// Check if this is an access token
    pub fn is_access(&self) -> bool {
        self.typ == "access"
    }

    /// Check if this is a refresh token
    pub fn is_refresh(&self) -> bool {
        self.typ == "refresh"
    }
}

/// JWT token manager
pub struct JwtManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtManager {
    /// Create a new JWT manager with the given secret
    pub fn new(secret: &str) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
        }
    }

    /// Generate a JWT token from claims
    pub fn generate(&self, claims: &Claims) -> Result<String, jsonwebtoken::errors::Error> {
        encode(&Header::default(), claims, &self.encoding_key)
    }

    /// Validate and decode a JWT token
    pub fn validate(&self, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let validation = Validation::default();
        let token_data = decode::<Claims>(token, &self.decoding_key, &validation)?;
        Ok(token_data.claims)
    }

    /// Generate access and refresh token pair
    pub fn generate_token_pair(
        &self,
        ai_id: &str,
        tier: &str,
    ) -> Result<TokenPair, jsonwebtoken::errors::Error> {
        let access_claims = Claims::new_access(ai_id, tier);
        let refresh_claims = Claims::new_refresh(ai_id, tier);

        Ok(TokenPair {
            access_token: self.generate(&access_claims)?,
            refresh_token: self.generate(&refresh_claims)?,
            expires_in: 3600, // 1 hour in seconds
            token_type: "Bearer".to_string(),
        })
    }
}

/// Token pair response
#[derive(Debug, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub token_type: String,
}

/// Password hashing using Argon2
pub mod password {
    use argon2::{
        password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
        Argon2,
    };

    /// Hash a password using Argon2
    pub fn hash(password: &str) -> Result<String, argon2::password_hash::Error> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2.hash_password(password.as_bytes(), &salt)?;
        Ok(hash.to_string())
    }

    /// Verify a password against a hash
    pub fn verify(password: &str, hash: &str) -> Result<bool, argon2::password_hash::Error> {
        let parsed_hash = PasswordHash::new(hash)?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }
}

/// Authenticated identity extracted from request
#[derive(Debug, Clone)]
pub struct AuthenticatedAi {
    /// AI identifier
    pub ai_id: String,
    /// Rate limit tier
    pub tier: String,
    /// Authentication method used
    pub auth_method: AuthMethod,
}

#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// JWT Bearer token
    Jwt,
    /// API Key
    ApiKey,
}

/// Extract bearer token from Authorization header
pub fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    auth_header
        .strip_prefix("Bearer ")
        .or_else(|| auth_header.strip_prefix("bearer "))
}

/// Extract API key from X-API-Key header or query param
pub fn extract_api_key(headers: &axum::http::HeaderMap, query: Option<&str>) -> Option<String> {
    // Check header first
    if let Some(key) = headers.get("X-API-Key").and_then(|v| v.to_str().ok()) {
        return Some(key.to_string());
    }

    // Fall back to query parameter
    query.map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_roundtrip() {
        let manager = JwtManager::new("test-secret");
        let claims = Claims::new_access("lyra-584", "pro");

        let token = manager.generate(&claims).unwrap();
        let decoded = manager.validate(&token).unwrap();

        assert_eq!(decoded.sub, "lyra-584");
        assert_eq!(decoded.tier, "pro");
        assert!(decoded.is_access());
    }

    #[test]
    fn test_password_hash_verify() {
        let password = "secure-password-123";
        let hash = password::hash(password).unwrap();

        assert!(password::verify(password, &hash).unwrap());
        assert!(!password::verify("wrong-password", &hash).unwrap());
    }
}
