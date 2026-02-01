//! Firebase CLI for AI-Foundation
//!
//! Provides CLI access to Firebase services:
//! - Crashlytics: View crash reports, issues, and trends
//! - Firestore: Read-only document access
//! - Auth: User lookup
//! - Remote Config: Get/set configuration
//!
//! Part of AI-Foundation - True AI Empowerment

pub mod auth;
pub mod client;
pub mod crashlytics;
pub mod error;
pub mod firestore;

pub use auth::FirebaseAuth;
pub use client::FirebaseClient;
pub use crashlytics::CrashlyticsClient;
pub use error::{FirebaseError, Result};
pub use firestore::FirestoreClient;
pub mod play_vitals;
pub use play_vitals::PlayVitalsClient;
