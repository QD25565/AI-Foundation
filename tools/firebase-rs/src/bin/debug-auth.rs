//! Debug authentication flow - print raw response

use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
struct ServiceAccount {
    project_id: String,
    private_key_id: String,
    private_key: String,
    client_email: String,
    token_uri: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Firebase Auth Debug (Raw) ===\n");
    
    // Load service account
    let sa_json = std::fs::read_to_string(std::env::var("GOOGLE_APPLICATION_CREDENTIALS").expect("GOOGLE_APPLICATION_CREDENTIALS must be set").as_str())?;
    let sa: ServiceAccount = serde_json::from_str(&sa_json)?;
    
    println!("Project: {}", sa.project_id);
    println!("Email: {}", sa.client_email);
    println!("Token URI: {}\n", sa.token_uri);
    
    // Create JWT
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    
    let header = serde_json::json!({
        "alg": "RS256",
        "typ": "JWT",
        "kid": sa.private_key_id
    });
    
    let claims = serde_json::json!({
        "iss": sa.client_email,
        "sub": sa.client_email,
        "aud": sa.token_uri,
        "iat": now,
        "exp": now + 3600,
        "scope": "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/datastore"
    });
    
    println!("JWT Claims: {}\n", serde_json::to_string_pretty(&claims)?);
    
    // Sign JWT
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    let mut jwt_header = Header::new(Algorithm::RS256);
    jwt_header.kid = Some(sa.private_key_id.clone());
    
    let key = EncodingKey::from_rsa_pem(sa.private_key.as_bytes())?;
    let jwt = encode(&jwt_header, &claims, &key)?;
    
    println!("JWT created (length: {})\n", jwt.len());
    
    // Exchange for token
    let client = reqwest::Client::new();
    let response = client
        .post(&sa.token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &jwt),
        ])
        .send()
        .await?;
    
    let status = response.status();
    let body = response.text().await?;
    
    println!("Response Status: {}", status);
    println!("Response Body:\n{}\n", body);
    
    // Parse and show
    let token_resp: TokenResponse = serde_json::from_str(&body)?;
    println!("Parsed response:");
    println!("  access_token: {:?}", token_resp.access_token.as_ref().map(|t| format!("{}...", &t[..20.min(t.len())])));
    println!("  id_token: {:?}", token_resp.id_token.as_ref().map(|t| format!("{}...", &t[..20.min(t.len())])));
    println!("  expires_in: {:?}", token_resp.expires_in);
    println!("  token_type: {:?}", token_resp.token_type);
    
    Ok(())
}
