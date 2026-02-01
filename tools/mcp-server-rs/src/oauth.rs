//! OAuth 2.1 Implementation for MCP Server
//!
//! Implements:
//! - RFC 9728: OAuth 2.0 Protected Resource Metadata
//! - RFC 8414: OAuth 2.0 Authorization Server Metadata
//! - RFC 7591: OAuth 2.0 Dynamic Client Registration
//! - OAuth 2.1 with PKCE (RFC 7636)
//!
//! This provides Claude.ai custom connector compatibility.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use axum::{
    extract::{Query, State, Form},
    http::{StatusCode, header, HeaderMap},
    response::{Html, IntoResponse, Redirect, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Sha256, Digest};

// ============================================================================
// Configuration
// ============================================================================

/// OAuth server configuration
#[derive(Clone)]
pub struct OAuthConfig {
    /// The issuer URL (e.g., https://mcp.myappapp.org)
    pub issuer: String,
    /// Access token lifetime in seconds (default: 1 hour)
    pub access_token_lifetime: u64,
    /// Refresh token lifetime in seconds (default: 30 days)
    pub refresh_token_lifetime: u64,
    /// Authorization code lifetime in seconds (default: 5 minutes)
    pub auth_code_lifetime: u64,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            issuer: "https://mcp.myappapp.org".to_string(),
            access_token_lifetime: 3600,        // 1 hour
            refresh_token_lifetime: 2592000,    // 30 days
            auth_code_lifetime: 300,            // 5 minutes
        }
    }
}

// ============================================================================
// Data Structures
// ============================================================================

/// Registered OAuth client (from DCR)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisteredClient {
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub registration_access_token: String,
    pub registration_client_uri: String,
    pub client_id_issued_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret_expires_at: Option<u64>,
}

/// Authorization code with PKCE
#[derive(Clone, Debug)]
pub struct AuthorizationCode {
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub user_id: String,
    pub expires_at: u64,
}

/// Access token
#[derive(Clone, Debug)]
pub struct AccessToken {
    pub token: String,
    pub client_id: String,
    pub user_id: String,
    pub scope: String,
    pub expires_at: u64,
}

/// Refresh token
#[derive(Clone, Debug)]
pub struct RefreshToken {
    pub token: String,
    pub client_id: String,
    pub user_id: String,
    pub scope: String,
    pub expires_at: u64,
}

/// OAuth state storage (in-memory, production should use persistent storage)
pub struct OAuthState {
    pub config: OAuthConfig,
    pub clients: HashMap<String, RegisteredClient>,
    pub auth_codes: HashMap<String, AuthorizationCode>,
    pub access_tokens: HashMap<String, AccessToken>,
    pub refresh_tokens: HashMap<String, RefreshToken>,
}

impl OAuthState {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            clients: HashMap::new(),
            auth_codes: HashMap::new(),
            access_tokens: HashMap::new(),
            refresh_tokens: HashMap::new(),
        }
    }

    /// Pre-register a client to bypass broken DCR flow in Claude.ai
    ///
    /// This is a workaround for https://github.com/anthropics/claude-code/issues/11814
    /// where Claude.ai's automatic OAuth DCR discovery fails. Users can manually
    /// enter these credentials in Claude.ai Settings > Connectors > Advanced settings.
    ///
    /// Credentials:
    ///   client_id: cove_claude_web
    ///   client_secret: aif_2025_mcp_secret_xK9mP2nQ8vL4
    pub fn pre_register_claude_client(&mut self) {
        let now = Self::now();
        let issuer = self.config.issuer.clone();

        // Pre-registered client for Claude.ai web (Cove)
        let client = RegisteredClient {
            client_id: "cove_claude_web".to_string(),
            client_secret: Some("aif_2025_mcp_secret_xK9mP2nQ8vL4".to_string()),
            client_name: "Claude.ai Web Connector".to_string(),
            redirect_uris: vec![
                "https://claude.ai/oauth/callback".to_string(),
                "https://claude.ai/api/oauth/callback".to_string(),
                "https://www.claude.ai/oauth/callback".to_string(),
                // Localhost for testing
                "http://localhost:3000/oauth/callback".to_string(),
            ],
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            token_endpoint_auth_method: "client_secret_basic".to_string(),
            registration_access_token: Self::generate_token(),
            registration_client_uri: format!("{}/register/cove_claude_web", issuer),
            client_id_issued_at: now,
            client_secret_expires_at: None, // Never expires
        };

        self.clients.insert(client.client_id.clone(), client);
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn generate_token() -> String {
        let bytes: [u8; 32] = rand::random();
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn generate_client_id() -> String {
        let bytes: [u8; 16] = rand::random();
        format!("mcp_{}", URL_SAFE_NO_PAD.encode(bytes))
    }

    fn generate_client_secret() -> String {
        let bytes: [u8; 32] = rand::random();
        URL_SAFE_NO_PAD.encode(bytes)
    }
}

// ============================================================================
// RFC 9728: Protected Resource Metadata
// ============================================================================

/// Protected Resource Metadata (RFC 9728)
#[derive(Serialize)]
pub struct ProtectedResourceMetadata {
    /// The resource identifier (the MCP server URL)
    pub resource: String,
    /// List of authorization servers that can issue tokens for this resource
    pub authorization_servers: Vec<String>,
    /// Scopes supported by this resource
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes_supported: Option<Vec<String>>,
    /// Bearer token methods supported
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer_methods_supported: Option<Vec<String>>,
    /// Resource documentation URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_documentation: Option<String>,
}

/// GET /.well-known/oauth-protected-resource
pub async fn protected_resource_metadata(
    State(state): State<Arc<RwLock<OAuthState>>>,
) -> impl IntoResponse {
    let state = state.read().await;
    let issuer = &state.config.issuer;

    let metadata = ProtectedResourceMetadata {
        resource: format!("{}/mcp", issuer),
        authorization_servers: vec![issuer.clone()],
        scopes_supported: Some(vec![
            "mcp:tools".to_string(),
            "notebook:read".to_string(),
            "notebook:write".to_string(),
        ]),
        bearer_methods_supported: Some(vec!["header".to_string()]),
        resource_documentation: Some(format!("{}/docs", issuer)),
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string_pretty(&metadata).unwrap(),
    )
}

// ============================================================================
// RFC 8414: Authorization Server Metadata
// ============================================================================

/// Authorization Server Metadata (RFC 8414)
#[derive(Serialize)]
pub struct AuthorizationServerMetadata {
    /// Issuer identifier
    pub issuer: String,
    /// Authorization endpoint
    pub authorization_endpoint: String,
    /// Token endpoint
    pub token_endpoint: String,
    /// Dynamic client registration endpoint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    /// Scopes supported
    pub scopes_supported: Vec<String>,
    /// Response types supported
    pub response_types_supported: Vec<String>,
    /// Grant types supported
    pub grant_types_supported: Vec<String>,
    /// Token endpoint auth methods
    pub token_endpoint_auth_methods_supported: Vec<String>,
    /// Code challenge methods (PKCE)
    pub code_challenge_methods_supported: Vec<String>,
    /// Service documentation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_documentation: Option<String>,
}

/// GET /.well-known/oauth-authorization-server
pub async fn authorization_server_metadata(
    State(state): State<Arc<RwLock<OAuthState>>>,
) -> impl IntoResponse {
    let state = state.read().await;
    let issuer = &state.config.issuer;

    let metadata = AuthorizationServerMetadata {
        issuer: issuer.clone(),
        authorization_endpoint: format!("{}/authorize", issuer),
        token_endpoint: format!("{}/token", issuer),
        registration_endpoint: Some(format!("{}/register", issuer)),
        scopes_supported: vec![
            "mcp:tools".to_string(),
            "notebook:read".to_string(),
            "notebook:write".to_string(),
        ],
        response_types_supported: vec!["code".to_string()],
        grant_types_supported: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        token_endpoint_auth_methods_supported: vec![
            "client_secret_basic".to_string(),
            "client_secret_post".to_string(),
            "none".to_string(),
        ],
        code_challenge_methods_supported: vec![
            "S256".to_string(),
            "plain".to_string(),
        ],
        service_documentation: Some(format!("{}/docs", issuer)),
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string_pretty(&metadata).unwrap(),
    )
}

// ============================================================================
// RFC 7591: Dynamic Client Registration
// ============================================================================

/// Client registration request
#[derive(Debug, Deserialize)]
pub struct ClientRegistrationRequest {
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub grant_types: Option<Vec<String>>,
    #[serde(default)]
    pub response_types: Option<Vec<String>>,
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
}

/// POST /register - Dynamic Client Registration
pub async fn register_client(
    State(state): State<Arc<RwLock<OAuthState>>>,
    Json(request): Json<ClientRegistrationRequest>,
) -> Response {
    let mut state = state.write().await;
    let issuer = state.config.issuer.clone();

    // Validate redirect URIs
    if request.redirect_uris.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_redirect_uri",
                "error_description": "At least one redirect_uri is required"
            })),
        ).into_response();
    }

    // Validate redirect URIs are HTTPS (except localhost for development)
    for uri in &request.redirect_uris {
        if !uri.starts_with("https://") && !uri.starts_with("http://localhost") && !uri.starts_with("http://127.0.0.1") {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_redirect_uri",
                    "error_description": "redirect_uri must use HTTPS"
                })),
            ).into_response();
        }
    }

    let client_id = OAuthState::generate_client_id();
    let client_secret = OAuthState::generate_client_secret();
    let registration_token = OAuthState::generate_token();
    let now = OAuthState::now();

    let grant_types = request.grant_types.unwrap_or_else(|| {
        vec!["authorization_code".to_string(), "refresh_token".to_string()]
    });
    let response_types = request.response_types.unwrap_or_else(|| {
        vec!["code".to_string()]
    });
    let auth_method = request.token_endpoint_auth_method
        .unwrap_or_else(|| "client_secret_basic".to_string());

    let client = RegisteredClient {
        client_id: client_id.clone(),
        client_secret: Some(client_secret.clone()),
        client_name: request.client_name.unwrap_or_else(|| "MCP Client".to_string()),
        redirect_uris: request.redirect_uris,
        grant_types,
        response_types,
        token_endpoint_auth_method: auth_method,
        registration_access_token: registration_token,
        registration_client_uri: format!("{}/register/{}", issuer, client_id),
        client_id_issued_at: now,
        client_secret_expires_at: None, // Never expires
    };

    let response_client = client.clone();
    state.clients.insert(client_id, client);

    (
        StatusCode::CREATED,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string_pretty(&response_client).unwrap(),
    ).into_response()
}

// ============================================================================
// Authorization Endpoint
// ============================================================================

/// Authorization request query parameters
#[derive(Debug, Deserialize)]
pub struct AuthorizationRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

/// GET /authorize - Authorization endpoint
/// For MCP, we auto-approve since the user already consented by adding the connector
pub async fn authorize(
    State(state): State<Arc<RwLock<OAuthState>>>,
    Query(params): Query<AuthorizationRequest>,
) -> Response {
    let mut state = state.write().await;

    // Validate client exists
    let client = match state.clients.get(&params.client_id) {
        Some(c) => c.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Html(format!(r#"
                    <!DOCTYPE html>
                    <html>
                    <head><title>Error</title></head>
                    <body>
                        <h1>Authorization Error</h1>
                        <p>Unknown client: {}</p>
                    </body>
                    </html>
                "#, params.client_id)),
            ).into_response();
        }
    };

    // Validate redirect URI
    if !client.redirect_uris.contains(&params.redirect_uri) {
        return (
            StatusCode::BAD_REQUEST,
            Html(format!(r#"
                <!DOCTYPE html>
                <html>
                <head><title>Error</title></head>
                <body>
                    <h1>Authorization Error</h1>
                    <p>Invalid redirect_uri</p>
                </body>
                </html>
            "#)),
        ).into_response();
    }

    // Validate response_type
    if params.response_type != "code" {
        return redirect_error(
            &params.redirect_uri,
            "unsupported_response_type",
            "Only 'code' response type is supported",
            params.state.as_deref(),
        );
    }

    // Validate PKCE
    if params.code_challenge_method != "S256" && params.code_challenge_method != "plain" {
        return redirect_error(
            &params.redirect_uri,
            "invalid_request",
            "code_challenge_method must be S256 or plain",
            params.state.as_deref(),
        );
    }

    // Generate authorization code
    let code = OAuthState::generate_token();
    let scope = params.scope.unwrap_or_else(|| "mcp:tools".to_string());
    let expires_at = OAuthState::now() + state.config.auth_code_lifetime;

    // For MCP servers, we auto-approve (the user consented by adding the connector)
    // In a full implementation, this would show a consent screen
    let auth_code = AuthorizationCode {
        code: code.clone(),
        client_id: params.client_id,
        redirect_uri: params.redirect_uri.clone(),
        scope,
        code_challenge: params.code_challenge,
        code_challenge_method: params.code_challenge_method,
        user_id: "mcp_user".to_string(), // Auto-approved user
        expires_at,
    };

    state.auth_codes.insert(code.clone(), auth_code);

    // Redirect back with code
    let mut redirect_url = params.redirect_uri;
    redirect_url.push_str(if redirect_url.contains('?') { "&" } else { "?" });
    redirect_url.push_str(&format!("code={}", code));
    if let Some(state_param) = params.state {
        redirect_url.push_str(&format!("&state={}", state_param));
    }

    Redirect::temporary(&redirect_url).into_response()
}

fn redirect_error(redirect_uri: &str, error: &str, description: &str, state: Option<&str>) -> Response {
    let mut url = redirect_uri.to_string();
    url.push_str(if url.contains('?') { "&" } else { "?" });
    url.push_str(&format!("error={}&error_description={}", error, urlencoding::encode(description)));
    if let Some(s) = state {
        url.push_str(&format!("&state={}", s));
    }
    Redirect::temporary(&url).into_response()
}

// ============================================================================
// Token Endpoint
// ============================================================================

/// Token request
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub code_verifier: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// Token response
#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Token error response
#[derive(Serialize)]
pub struct TokenErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

/// POST /token - Token endpoint
pub async fn token(
    State(state): State<Arc<RwLock<OAuthState>>>,
    headers: HeaderMap,
    Form(request): Form<TokenRequest>,
) -> Response {
    let mut state = state.write().await;

    // Extract client credentials (from header or body)
    let (client_id, client_secret) = extract_client_credentials(&headers, &request);

    match request.grant_type.as_str() {
        "authorization_code" => {
            handle_authorization_code_grant(&mut state, client_id, client_secret, request).await
        }
        "refresh_token" => {
            handle_refresh_token_grant(&mut state, client_id, client_secret, request).await
        }
        _ => {
            (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "unsupported_grant_type".to_string(),
                    error_description: Some("Grant type not supported".to_string()),
                }),
            ).into_response()
        }
    }
}

fn extract_client_credentials(headers: &HeaderMap, request: &TokenRequest) -> (Option<String>, Option<String>) {
    // Try Basic auth header first
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        if let Ok(auth_str) = auth.to_str() {
            if auth_str.starts_with("Basic ") {
                if let Ok(decoded) = URL_SAFE_NO_PAD.decode(&auth_str[6..]) {
                    if let Ok(creds) = String::from_utf8(decoded) {
                        let parts: Vec<&str> = creds.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            return (Some(parts[0].to_string()), Some(parts[1].to_string()));
                        }
                    }
                }
            }
        }
    }

    // Fall back to request body
    (request.client_id.clone(), request.client_secret.clone())
}

async fn handle_authorization_code_grant(
    state: &mut OAuthState,
    client_id: Option<String>,
    _client_secret: Option<String>,
    request: TokenRequest,
) -> Response {
    let code = match &request.code {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_request".to_string(),
                    error_description: Some("Missing code parameter".to_string()),
                }),
            ).into_response();
        }
    };

    // Look up and remove the auth code
    let auth_code = match state.auth_codes.remove(code) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_grant".to_string(),
                    error_description: Some("Invalid or expired authorization code".to_string()),
                }),
            ).into_response();
        }
    };

    // Check expiration
    if OAuthState::now() > auth_code.expires_at {
        return (
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Authorization code expired".to_string()),
            }),
        ).into_response();
    }

    // Verify client_id matches
    if let Some(ref cid) = client_id {
        if cid != &auth_code.client_id {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_client".to_string(),
                    error_description: Some("Client ID mismatch".to_string()),
                }),
            ).into_response();
        }
    }

    // Verify redirect_uri matches
    if let Some(ref uri) = request.redirect_uri {
        if uri != &auth_code.redirect_uri {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_grant".to_string(),
                    error_description: Some("redirect_uri mismatch".to_string()),
                }),
            ).into_response();
        }
    }

    // Verify PKCE code_verifier
    let verifier = match &request.code_verifier {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_request".to_string(),
                    error_description: Some("Missing code_verifier".to_string()),
                }),
            ).into_response();
        }
    };

    let valid = match auth_code.code_challenge_method.as_str() {
        "S256" => {
            let mut hasher = Sha256::new();
            hasher.update(verifier.as_bytes());
            let hash = hasher.finalize();
            let computed = URL_SAFE_NO_PAD.encode(hash);
            computed == auth_code.code_challenge
        }
        "plain" => verifier == &auth_code.code_challenge,
        _ => false,
    };

    if !valid {
        return (
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Invalid code_verifier".to_string()),
            }),
        ).into_response();
    }

    // Issue tokens
    let access_token = OAuthState::generate_token();
    let refresh_token = OAuthState::generate_token();
    let now = OAuthState::now();

    state.access_tokens.insert(access_token.clone(), AccessToken {
        token: access_token.clone(),
        client_id: auth_code.client_id.clone(),
        user_id: auth_code.user_id.clone(),
        scope: auth_code.scope.clone(),
        expires_at: now + state.config.access_token_lifetime,
    });

    state.refresh_tokens.insert(refresh_token.clone(), RefreshToken {
        token: refresh_token.clone(),
        client_id: auth_code.client_id,
        user_id: auth_code.user_id,
        scope: auth_code.scope.clone(),
        expires_at: now + state.config.refresh_token_lifetime,
    });

    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")],
        Json(TokenResponse {
            access_token,
            token_type: "Bearer".to_string(),
            expires_in: state.config.access_token_lifetime,
            refresh_token: Some(refresh_token),
            scope: Some(auth_code.scope),
        }),
    ).into_response()
}

async fn handle_refresh_token_grant(
    state: &mut OAuthState,
    client_id: Option<String>,
    _client_secret: Option<String>,
    request: TokenRequest,
) -> Response {
    let token = match &request.refresh_token {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_request".to_string(),
                    error_description: Some("Missing refresh_token".to_string()),
                }),
            ).into_response();
        }
    };

    let refresh = match state.refresh_tokens.get(token) {
        Some(r) => r.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_grant".to_string(),
                    error_description: Some("Invalid refresh token".to_string()),
                }),
            ).into_response();
        }
    };

    // Check expiration
    if OAuthState::now() > refresh.expires_at {
        state.refresh_tokens.remove(token);
        return (
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Refresh token expired".to_string()),
            }),
        ).into_response();
    }

    // Verify client_id
    if let Some(ref cid) = client_id {
        if cid != &refresh.client_id {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_client".to_string(),
                    error_description: Some("Client ID mismatch".to_string()),
                }),
            ).into_response();
        }
    }

    // Issue new access token
    let new_access_token = OAuthState::generate_token();
    let now = OAuthState::now();

    state.access_tokens.insert(new_access_token.clone(), AccessToken {
        token: new_access_token.clone(),
        client_id: refresh.client_id.clone(),
        user_id: refresh.user_id,
        scope: refresh.scope.clone(),
        expires_at: now + state.config.access_token_lifetime,
    });

    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")],
        Json(TokenResponse {
            access_token: new_access_token,
            token_type: "Bearer".to_string(),
            expires_in: state.config.access_token_lifetime,
            refresh_token: None, // Keep using the same refresh token
            scope: Some(refresh.scope),
        }),
    ).into_response()
}

// ============================================================================
// Token Validation Middleware
// ============================================================================

/// Validates a Bearer token and returns the associated user/scope
pub async fn validate_token(
    state: &Arc<RwLock<OAuthState>>,
    headers: &HeaderMap,
) -> Result<(String, String), (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Missing Authorization header".to_string()))?;

    if !auth_header.starts_with("Bearer ") {
        return Err((StatusCode::UNAUTHORIZED, "Invalid token type, expected Bearer".to_string()));
    }

    let token = &auth_header[7..];
    let state = state.read().await;

    let access_token = state.access_tokens.get(token)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Invalid access token".to_string()))?;

    if OAuthState::now() > access_token.expires_at {
        return Err((StatusCode::UNAUTHORIZED, "Access token expired".to_string()));
    }

    Ok((access_token.user_id.clone(), access_token.scope.clone()))
}

/// Returns a 401 response with WWW-Authenticate header (RFC 9728 compliant)
pub fn unauthorized_response(issuer: &str, error: &str, description: &str) -> Response {
    let www_auth = format!(
        r#"Bearer resource_metadata="{issuer}/.well-known/oauth-protected-resource", error="{error}", error_description="{description}""#,
        issuer = issuer,
        error = error,
        description = description
    );

    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, www_auth)],
        description.to_string(),
    ).into_response()
}

// ============================================================================
// Utility: Random bytes (simple implementation)
// ============================================================================

mod rand {
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::cell::Cell;

    thread_local! {
        static STATE: Cell<u64> = Cell::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
        );
    }

    /// Simple xorshift64 PRNG - NOT cryptographically secure
    /// For production, use the `rand` crate with a proper CSPRNG
    fn next_u64() -> u64 {
        STATE.with(|s| {
            let mut x = s.get();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            // Mix with current time for additional entropy
            x = x.wrapping_add(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64
            );
            s.set(x);
            x
        })
    }

    pub fn random<const N: usize>() -> [u8; N] {
        let mut result = [0u8; N];
        for chunk in result.chunks_mut(8) {
            let rand = next_u64().to_le_bytes();
            chunk.copy_from_slice(&rand[..chunk.len()]);
        }
        result
    }
}
