use pyo3::prelude::*;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey, Signature, SecretKey};
use sha3::{Sha3_256, Digest};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use rand::{RngCore, rngs::OsRng};

// Global identity cache
static IDENTITY_CACHE: Lazy<Mutex<Option<AIIdentity>>> = Lazy::new(|| Mutex::new(None));

/// AI Identity with Ed25519 cryptographic signing
#[pyclass]
#[derive(Clone)]
pub struct AIIdentity {
    #[pyo3(get)]
    ai_id: String,
    #[pyo3(get)]
    display_name: String,
    #[pyo3(get)]
    fingerprint: String,
    #[pyo3(get)]
    public_key_hex: String,
    signing_key: Vec<u8>, // Private key bytes
}

#[pymethods]
impl AIIdentity {
    #[new]
    fn new(ai_id: String, display_name: String, signing_key_hex: String) -> PyResult<Self> {
        let signing_key_bytes = hex::decode(&signing_key_hex)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid signing key hex: {}", e)
            ))?;

        let signing_key_array: [u8; 32] = signing_key_bytes.clone().try_into()
            .map_err(|_| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Signing key must be 32 bytes"
            ))?;

        let signing_key = SigningKey::from_bytes(&signing_key_array);

        let verifying_key = signing_key.verifying_key();
        let public_key_bytes = verifying_key.to_bytes();
        let public_key_hex = hex::encode(public_key_bytes);

        // Generate fingerprint: SHA3-256 of public key, first 16 hex chars
        let mut hasher = Sha3_256::new();
        hasher.update(&public_key_bytes);
        let hash = hasher.finalize();
        let fingerprint = hex::encode(&hash[..8]).to_uppercase();

        Ok(AIIdentity {
            ai_id,
            display_name,
            fingerprint,
            public_key_hex,
            signing_key: signing_key_bytes,
        })
    }

    /// Sign a message with Ed25519
    fn sign(&self, message: &str) -> PyResult<String> {
        let signing_key = SigningKey::from_bytes(
            &self.signing_key.clone().try_into()
                .map_err(|_| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Invalid signing key"
                ))?
        );

        let signature = signing_key.sign(message.as_bytes());
        Ok(hex::encode(signature.to_bytes()))
    }

    /// Verify a signature
    #[staticmethod]
    fn verify(public_key_hex: &str, message: &str, signature_hex: &str) -> PyResult<bool> {
        let public_key_bytes = hex::decode(public_key_hex)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid public key hex: {}", e)
            ))?;

        let verifying_key = VerifyingKey::from_bytes(
            &public_key_bytes.try_into()
                .map_err(|_| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Public key must be 32 bytes"
                ))?
        ).map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
            format!("Invalid verifying key: {}", e)
        ))?;

        let signature_bytes = hex::decode(signature_hex)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid signature hex: {}", e)
            ))?;

        let signature = Signature::from_bytes(
            &signature_bytes.try_into()
                .map_err(|_| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Signature must be 64 bytes"
                ))?
        );

        Ok(verifying_key.verify_strict(message.as_bytes(), &signature).is_ok())
    }

    /// Generate a new random identity
    #[staticmethod]
    fn generate(ai_id: String, display_name: String) -> PyResult<Self> {
        let mut secret_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let signing_key_hex = hex::encode(signing_key.to_bytes());

        AIIdentity::new(ai_id, display_name, signing_key_hex)
    }

    fn __repr__(&self) -> String {
        format!(
            "AIIdentity(ai_id='{}', display_name='{}', fingerprint='{}')",
            self.ai_id, self.display_name, self.fingerprint
        )
    }
}

/// Fast path resolution
#[pyfunction]
fn normalize_path(path: &str) -> PyResult<String> {
    use std::path::PathBuf;
    let path = PathBuf::from(path);

    // Fast path: absolute path, no normalization needed
    if path.is_absolute() && !path.to_string_lossy().contains("..") {
        return Ok(path.to_string_lossy().to_string());
    }

    // Normalize path
    let normalized = path.canonicalize()
        .unwrap_or_else(|_| {
            // If canonicalize fails, do basic normalization
            let mut components = Vec::new();
            for component in path.components() {
                match component {
                    std::path::Component::ParentDir => { components.pop(); },
                    std::path::Component::CurDir => {},
                    c => components.push(c),
                }
            }
            components.iter().collect()
        });

    Ok(normalized.to_string_lossy().to_string())
}

/// SHA3-256 hash
#[pyfunction]
fn sha3_256(data: &str) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Get or create cached identity
#[pyfunction]
fn get_cached_identity() -> PyResult<Option<AIIdentity>> {
    let cache = IDENTITY_CACHE.lock().unwrap();
    Ok(cache.clone())
}

/// Cache identity globally
#[pyfunction]
fn cache_identity(identity: AIIdentity) -> PyResult<()> {
    let mut cache = IDENTITY_CACHE.lock().unwrap();
    *cache = Some(identity);
    Ok(())
}

#[pymodule]
fn ai_foundation_shared(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<AIIdentity>()?;
    m.add_function(wrap_pyfunction!(normalize_path, m)?)?;
    m.add_function(wrap_pyfunction!(sha3_256, m)?)?;
    m.add_function(wrap_pyfunction!(get_cached_identity, m)?)?;
    m.add_function(wrap_pyfunction!(cache_identity, m)?)?;
    Ok(())
}
