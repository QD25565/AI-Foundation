use firebase_rs::auth::FirebaseAuth;

#[tokio::main]
async fn main() {
    let auth = FirebaseAuth::from_file(std::env::var("GOOGLE_APPLICATION_CREDENTIALS").expect("GOOGLE_APPLICATION_CREDENTIALS must be set")).unwrap();
    match auth.get_access_token().await {
        Ok(token) => println!("Token: {}...{}", &token[..20], &token[token.len()-20..]),
        Err(e) => println!("Error: {:?}", e),
    }
}
