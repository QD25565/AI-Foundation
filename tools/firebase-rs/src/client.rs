//! Firebase REST API client
//!
//! Base client for making authenticated requests to Firebase APIs

use crate::auth::FirebaseAuth;
use crate::error::{FirebaseError, Result};
use reqwest::{Method, Response};
use serde::de::DeserializeOwned;
use std::sync::Arc;

/// Firebase REST API client with authentication
pub struct FirebaseClient {
    auth: Arc<FirebaseAuth>,
    http_client: reqwest::Client,
    project_id: String,
}

impl FirebaseClient {
    /// Create new Firebase client
    pub fn new(auth: FirebaseAuth) -> Self {
        let project_id = auth.project_id().to_string();
        Self {
            auth: Arc::new(auth),
            http_client: reqwest::Client::builder()
                .user_agent("firebase-cli/0.1.0 (AI-Foundation)")
                .build()
                .expect("Failed to create HTTP client"),
            project_id,
        }
    }

    /// Get project ID
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Make authenticated GET request
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.request(Method::GET, url, None::<&()>).await
    }

    /// Make authenticated POST request
    pub async fn post<T: serde::Serialize>(&self, url: &str, body: &T) -> Result<Response> {
        self.request(Method::POST, url, Some(body)).await
    }

    /// Make authenticated PATCH request
    pub async fn patch<T: serde::Serialize>(&self, url: &str, body: &T) -> Result<Response> {
        self.request(Method::PATCH, url, Some(body)).await
    }

    /// Make authenticated request with optional body
    async fn request<T: serde::Serialize>(
        &self,
        method: Method,
        url: &str,
        body: Option<&T>,
    ) -> Result<Response> {
        let token = self.auth.get_access_token().await?;

        let mut request = self
            .http_client
            .request(method, url)
            .bearer_auth(&token);

        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request.send().await?;

        // Handle common error status codes
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let status_code = status.as_u16();
        let body = response.text().await.unwrap_or_default();

        match status_code {
            401 => Err(FirebaseError::AuthError("Invalid or expired token".to_string())),
            403 => Err(FirebaseError::PermissionDenied(body)),
            404 => Err(FirebaseError::NotFound(body)),
            429 => {
                // Try to parse retry-after
                Err(FirebaseError::RateLimited(60))
            }
            _ => Err(FirebaseError::from_response(status_code, &body)),
        }
    }

    /// Make GET request and parse JSON response
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let response = self.get(url).await?;
        let text = response.text().await?;
        serde_json::from_str(&text).map_err(|e| {
            FirebaseError::ApiError {
                status: 0,
                message: format!("JSON parse error: {}. Response: {}", e, &text[..text.len().min(500)]),
            }
        })
    }

    /// Build Firebase REST API URL
    pub fn api_url(&self, service: &str, path: &str) -> String {
        match service {
            "crashlytics" => {
                format!(
                    "https://firebasecrashlytics.googleapis.com/v1beta1/projects/{}/{}",
                    self.project_id, path
                )
            }
            "firestore" => {
                format!(
                    "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents/{}",
                    self.project_id, path
                )
            }
            "auth" => {
                format!(
                    "https://identitytoolkit.googleapis.com/v1/projects/{}/{}",
                    self.project_id, path
                )
            }
            "config" => {
                format!(
                    "https://firebaseremoteconfig.googleapis.com/v1/projects/{}/remoteConfig",
                    self.project_id
                )
            }
            "management" => {
                format!(
                    "https://firebase.googleapis.com/v1beta1/projects/{}/{}",
                    self.project_id, path
                )
            }
            _ => panic!("Unknown Firebase service: {}", service),
        }
    }

    /// List Android apps in the Firebase project
    pub async fn list_android_apps(&self) -> Result<Vec<AndroidApp>> {
        let url = self.api_url("management", "androidApps");
        let response: AndroidAppsResponse = self.get_json(&url).await?;
        Ok(response.apps.unwrap_or_default())
    }
}

/// Android app info
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AndroidApp {
    /// Full resource name
    pub name: String,
    /// App ID (e.g., "1:123456789:android:abc123")
    #[serde(rename = "appId")]
    pub app_id: String,
    /// Display name
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    /// Package name
    #[serde(rename = "packageName")]
    pub package_name: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct AndroidAppsResponse {
    apps: Option<Vec<AndroidApp>>,
}
