# Post-Quantum Cryptography Transition Architecture

**Author:** Lyra | **Date:** 2026-03-01 | **Status:** Draft — Awaiting QD Review

## Background

QD approved PQC implementation (2026-03-01). Federation is experimental — better to build in now than retrofit later. Quantum computers capable of breaking Ed25519 (Shor's algorithm) are estimated 2030-2035. "Harvest now, decrypt later" attacks make this relevant today for signed messages.

## What Changes, What Stays

### Already Post-Quantum Safe (NO CHANGES)
- **XChaCha20-Poly1305** (engram/src/crypto.rs) — symmetric, PQ-safe
- **AES-256-GCM** (teamengram-rs/src/crypto.rs, afp-rs/src/keys.rs, passkey.rs) — symmetric, PQ-safe
- **HKDF-SHA256** (teamengram-rs/src/crypto.rs) — hash-based KDF, acceptable

### Must Migrate (Ed25519 → ML-DSA-65) — 12 Files (Resonance audit, corrected)
| # | File | Purpose | Ed25519 Operations |
|---|------|---------|-------------------|
| 1 | `federation-rs/src/identity.rs` | Teambook identity keypair | generate, sign, verify, persist (32-byte raw file) |
| 2 | `federation-rs/src/messages.rs` | Federation message signing | sign, verify (FederationMessage + FederatedPresence + SignedEvent) |
| 3 | `federation-rs/src/node.rs` | Node identity + verification | FederationNode holds VerifyingKey, custom serde hex encoding |
| 4 | `federation-rs/src/session.rs` | QUIC handshake | Hello/Welcome exchange, mutual Ed25519 signing via SigningKey |
| 5 | `federation-rs/src/lib.rs` | Helper functions | sign_data(), verify_signature(), node_id_from_pubkey() |
| 6 | `mcp-server-rs/src/identity_verify.rs` | Notebook access protection | derive_key, sign_challenge, verify (deterministic) |
| 7 | `afp-rs/src/keys.rs` | Key storage abstraction | KeyPair generate/sign/verify, KeyStorage trait, 3 backends |
| 8 | `afp-rs/src/identity.rs` | AIIdentity struct | verify, fingerprint, **CompactIdentity [u8;32] pubkey** |
| 9 | `afp-rs/src/message.rs` | AFP message protocol | AFPMessage **Signature field (64-byte serde)**, sign/verify via CBOR signable bytes |
| 10 | `afp-rs/src/tpm.rs` | TPM/DPAPI key storage | Seals Ed25519 private key bytes, StoredHID persists pubkey |
| 11 | `afp-rs/src/client.rs` | AFP client | Signs Hello, DMs, broadcasts, pings. Verifies all responses. |
| 12 | `afp-rs/src/server.rs` | AFP server | Verifies Hello, signs Welcome/Reject. Extracts client pubkey from CompactIdentity. |

**Removed:** `federation-rs/src/discovery/passkey.rs` — AES-256-GCM only, no Ed25519 (corrected by Resonance).

**Also noted:** `shared-rs/src/bin/identity-cli.rs` uses SHA3-256 for fingerprints while afp-rs and federation-rs use SHA-256. Should unify before migration.

### Hard Constraints (fixed-size assumptions that must change)
1. **`CompactIdentity.pubkey: [u8; 32]`** (afp-rs/src/identity.rs) — Ed25519 pubkey hardcoded to 32 bytes. ML-DSA-65 pubkey is 1,952 bytes. Must become `Vec<u8>` or enum.
2. **`AFPMessage.signature` serde** (afp-rs/src/message.rs:760) — Deserializer checks `bytes.len() != 64`, rejects anything else. Must accept variable-length.
3. **`FederationMessage.signature: Signature`** (federation-rs/src/messages.rs) — Direct Ed25519 `Signature` type. Must become `FederationSignature`.
4. **`FederatedPresence.signature: Signature`** (federation-rs/src/messages.rs) — Same issue.
5. **`identity.key` file format** (federation-rs/src/identity.rs:69) — Checks `bytes.len() != SECRET_KEY_LENGTH` (32). ML-DSA-65 secret key is 4,032 bytes.

### Transport Layer (STAYS Ed25519)
- **iroh 0.96** — QUIC transport uses Ed25519 internally. We cannot change iroh's crypto. PQC goes at the application layer ON TOP of iroh transport. This is acceptable: iroh's Ed25519 protects the transport session (ephemeral), our ML-DSA protects the application messages (persistent, archivable).

## Algorithm Selection

### Signatures: ML-DSA-65 (FIPS 204, Level 3)
- Public key: 1,952 bytes (vs Ed25519: 32 bytes)
- Secret key: 4,032 bytes (vs Ed25519: 64 bytes)
- Signature: 3,309 bytes (vs Ed25519: 64 bytes)
- Signing: ~587μs (vs Ed25519: ~16μs) — 37x slower, acceptable for our volume
- Verification: ~86μs (vs Ed25519: ~46μs) — 2x slower, acceptable

Why ML-DSA-65 over ML-DSA-44: Level 3 (192-bit) vs Level 2 (128-bit). Infrastructure meant to outlive us should target higher security margins.

### Key Encapsulation: ML-KEM-768 (FIPS 203, Level 3)
- Encapsulation key: 1,184 bytes
- Decapsulation key: 2,400 bytes
- Ciphertext: 1,088 bytes
- Shared secret: 32 bytes
- Used for: Federation session key establishment (replacing raw key sharing)

### Rust Crates
- `ml-dsa` v0.1.0-rc.7 (RustCrypto) — pure Rust, uses same `Signer`/`Verifier` traits as ed25519-dalek
- `ml-kem` v0.3.0-rc.0 (RustCrypto) — pure Rust
- Both are pre-release, NOT audited. Acceptable because federation itself is experimental.
- NOT pqcrypto (PQClean C wrapper, archived July 2026)

## Architecture Design

### Core Abstraction: `SignatureScheme`

```rust
/// Identifies which signature algorithm produced a signature.
/// Serialized as u8 in wire format for compactness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SignatureScheme {
    /// Classical Ed25519 (64-byte signatures)
    Ed25519 = 0,

    /// Post-quantum ML-DSA-65 (3,309-byte signatures)
    MlDsa65 = 1,

    /// Hybrid: Ed25519 + ML-DSA-65 (both must verify)
    /// Signature = Ed25519(64) || ML-DSA-65(3309) = 3,373 bytes
    HybridEd25519MlDsa65 = 2,
}
```

### Core Abstraction: `FederationSignature`

```rust
/// Algorithm-agile signature container.
/// Replaces raw `ed25519_dalek::Signature` in all federation structs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationSignature {
    /// Which algorithm(s) produced this signature
    pub scheme: SignatureScheme,

    /// Raw signature bytes (length depends on scheme)
    /// - Ed25519: 64 bytes
    /// - MlDsa65: 3,309 bytes
    /// - Hybrid: 64 + 3,309 = 3,373 bytes (Ed25519 first, then ML-DSA)
    pub bytes: Vec<u8>,
}

impl FederationSignature {
    /// Create an Ed25519-only signature (current behavior, Phase 1)
    pub fn ed25519(sig: ed25519_dalek::Signature) -> Self {
        Self {
            scheme: SignatureScheme::Ed25519,
            bytes: sig.to_bytes().to_vec(),
        }
    }

    /// Create a hybrid signature (Phase 2)
    pub fn hybrid(ed25519_sig: ed25519_dalek::Signature, ml_dsa_sig: &[u8]) -> Self {
        let mut bytes = Vec::with_capacity(64 + ml_dsa_sig.len());
        bytes.extend_from_slice(&ed25519_sig.to_bytes());
        bytes.extend_from_slice(ml_dsa_sig);
        Self {
            scheme: SignatureScheme::HybridEd25519MlDsa65,
            bytes,
        }
    }
}
```

### Core Abstraction: `FederationIdentity` (trait)

```rust
/// Trait abstracting over signature algorithms for federation identity.
/// TeambookIdentity implements this today with Ed25519.
/// Phase 2 adds HybridIdentity implementing both.
pub trait FederationSigner: Send + Sync {
    /// Sign arbitrary bytes, returning an algorithm-agile signature
    fn sign_federation(&self, data: &[u8]) -> FederationSignature;

    /// The signature scheme this signer produces
    fn scheme(&self) -> SignatureScheme;

    /// Public key material for verification (algorithm-specific)
    fn public_key_bytes(&self) -> Vec<u8>;

    /// Short hex identifier derived from public key
    fn short_id(&self) -> String;
}

/// Verify a FederationSignature against public key material.
/// Dispatches to the correct algorithm based on scheme.
pub fn verify_federation_signature(
    scheme: SignatureScheme,
    public_key_bytes: &[u8],
    data: &[u8],
    signature: &FederationSignature,
) -> bool {
    match scheme {
        SignatureScheme::Ed25519 => {
            // Current path
            let pubkey_arr: [u8; 32] = match public_key_bytes.try_into() {
                Ok(arr) => arr,
                Err(_) => return false,
            };
            let verifying_key = match ed25519_dalek::VerifyingKey::from_bytes(&pubkey_arr) {
                Ok(vk) => vk,
                Err(_) => return false,
            };
            let sig_arr: [u8; 64] = match signature.bytes[..].try_into() {
                Ok(arr) => arr,
                Err(_) => return false,
            };
            let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
            use ed25519_dalek::Verifier;
            verifying_key.verify(data, &sig).is_ok()
        }
        SignatureScheme::MlDsa65 => {
            // Phase 2: ML-DSA-65 verification
            todo!("ML-DSA-65 verification — Phase 2")
        }
        SignatureScheme::HybridEd25519MlDsa65 => {
            // Phase 2: Both must verify
            // Split signature.bytes into Ed25519 (first 64) and ML-DSA (remaining)
            // Verify BOTH — fail if either fails
            todo!("Hybrid verification — Phase 2")
        }
    }
}
```

### Wire Format Changes

#### FederationMessage (messages.rs)

```rust
// BEFORE (current):
pub struct FederationMessage {
    pub id: String,
    pub from: String,
    pub timestamp: DateTime<Utc>,
    pub payload: FederationPayload,
    #[serde(with = "signature_serde")]
    pub signature: Signature,  // ← hardcoded Ed25519
}

// AFTER (Phase 1):
pub struct FederationMessage {
    pub id: String,
    pub from: String,
    pub timestamp: DateTime<Utc>,
    pub payload: FederationPayload,
    pub signature: FederationSignature,  // ← algorithm-agile
}
```

#### SignedEvent (messages.rs)

```rust
// BEFORE:
pub struct SignedEvent {
    pub event_bytes: Vec<u8>,
    pub origin_pubkey: String,        // Ed25519 32-byte hex
    pub signature: String,            // Ed25519 64-byte hex
    pub content_id: String,
}

// AFTER (Phase 1):
pub struct SignedEvent {
    pub event_bytes: Vec<u8>,
    pub origin_pubkey: String,        // Stays hex-encoded, length varies by scheme
    pub signature: FederationSignature, // Algorithm-agile
    pub content_id: String,
    pub scheme: SignatureScheme,       // Tells receiver how to verify
}
```

#### FederatedPresence (messages.rs)

```rust
// BEFORE:
pub struct FederatedPresence {
    // ...
    #[serde(with = "signature_serde")]
    pub signature: Signature,  // ← hardcoded Ed25519
}

// AFTER (Phase 1):
pub struct FederatedPresence {
    // ...
    pub signature: FederationSignature,  // ← algorithm-agile
}
```

### Notebook Access Protection (identity_verify.rs)

This is a **separate concern** from federation. The notebook challenge-response system:
- Is local-only (never crosses network)
- Uses deterministic key derivation (device_secret + ai_id)
- Has a 1-second timing constraint

**Phase 1:** No change. Ed25519 is fine for local challenge-response — there is no "harvest now, decrypt later" risk since challenges are ephemeral and local.

**Phase 2:** Migrate to ML-DSA-65 for consistency. The deterministic key derivation changes from:
```
SHA256(device_secret || ai_id || "ed25519-signing-key") → Ed25519 seed
```
to:
```
SHA256(device_secret || ai_id || "ml-dsa-65-signing-key") → ML-DSA-65 seed
```

This is lower priority than federation — quantum computers breaking local challenge-response requires physical access to the machine AND a quantum computer, which is a much harder threat model.

### AFP Key Storage (afp-rs/src/keys.rs)

The `KeyPair` struct and `KeyStorage` trait are currently Ed25519-specific. Phase 1 adds algorithm agility:

```rust
// Phase 1: Algorithm-agile KeyPair
pub enum KeyMaterial {
    Ed25519(SigningKey),
    // Phase 2: MlDsa65(ml_dsa::MlDsa65SigningKey),
}

pub struct KeyPair {
    key: KeyMaterial,
}

impl KeyPair {
    pub fn sign_federation(&self, message: &[u8]) -> FederationSignature {
        match &self.key {
            KeyMaterial::Ed25519(sk) => {
                FederationSignature::ed25519(sk.sign(message))
            }
        }
    }
}
```

### Identity Key Persistence

Currently: `~/.ai-foundation/federation/identity.key` — raw 32-byte Ed25519 secret key.

**Phase 2:** New file format for ML-DSA-65 keys (4,032 bytes):
```
~/.ai-foundation/federation/identity.key     — Ed25519 (kept for backward compat)
~/.ai-foundation/federation/identity-pqc.key — ML-DSA-65
```

Both files loaded at startup. Hybrid identity signs with both keys. Once Ed25519 is dropped (Phase 3), only `identity-pqc.key` remains.

## Implementation Phases

### Phase 1: Algorithm Agility (NOW — this PR)
**Goal:** Make all signing/verification algorithm-agnostic without adding ML-DSA dependency.

Files to modify:
1. `federation-rs/src/lib.rs` — Add `SignatureScheme`, `FederationSignature`, `verify_federation_signature()`
2. `federation-rs/src/messages.rs` — Replace `Signature` with `FederationSignature` in FederationMessage, FederatedPresence, SignedEvent. Remove `signature_serde` (replaced by FederationSignature's own serde).
3. `federation-rs/src/identity.rs` — Implement `FederationSigner` trait for `TeambookIdentity`
4. `federation-rs/src/node.rs` — Update `verify_signature` to use `FederationSignature`. Update VerifyingKey serde to support variable-length pubkeys.
5. `federation-rs/src/session.rs` — Update handshake to use `FederationSigner` trait instead of raw `SigningKey`
6. `afp-rs/src/identity.rs` — `CompactIdentity.pubkey: [u8; 32]` → `Vec<u8>` + `scheme: SignatureScheme`. Update `AIIdentity.verify()`.
7. `afp-rs/src/message.rs` — `AFPMessage.signature: Signature` → `FederationSignature`. Remove 64-byte serde check. Update `sign()`/`verify()`.
8. `afp-rs/src/keys.rs` — Add `KeyMaterial` enum wrapping `SigningKey`. `KeyPair::sign()` returns `FederationSignature`.
9. `afp-rs/src/tpm.rs` — Update `StoredHID.pubkey_hex` documentation for variable-length. DPAPI seal/unseal stays (still sealing Ed25519 bytes in Phase 1).
10. `afp-rs/src/client.rs` — Update signing calls to use `FederationSignature`
11. `afp-rs/src/server.rs` — Update verification calls to use `FederationSignature`

**NOT in Phase 1:** `mcp-server-rs/src/identity_verify.rs` (local-only, no wire format concerns, defer to Phase 2).

**All existing behavior preserved.** Ed25519 is the only active scheme. The agility layer is plumbing for Phase 2.

### Phase 2: Hybrid Signatures (when ml-dsa 0.1.0 stable or QD approves rc)
**Goal:** Every federation message signed with BOTH Ed25519 AND ML-DSA-65.

New dependencies:
```toml
ml-dsa = "0.1"    # FIPS 204 signatures
ml-kem = "0.3"    # FIPS 203 key encapsulation (federation key exchange)
```

Changes:
1. Add `HybridIdentity` — holds both Ed25519 and ML-DSA-65 keypairs
2. Generate + persist ML-DSA-65 key alongside Ed25519
3. `FederationSigner::sign_federation()` produces hybrid signatures
4. `verify_federation_signature()` checks BOTH (fail if either fails)
5. ML-KEM-768 for federation session key establishment
6. Migrate `identity_verify.rs` to ML-DSA-65

### Phase 3: Drop Ed25519 (2027+ after PQC crate audit)
**Goal:** Remove classical crypto from signing path.

Changes:
1. `SignatureScheme::Ed25519` accepted for verification only (legacy messages)
2. New signatures are ML-DSA-65 only
3. Ed25519 key material kept for iroh transport compatibility
4. `identity_verify.rs` ML-DSA-65 only

## Size Impact Analysis

Federation messages currently: ~200-500 bytes typical (CBOR payload + 64-byte Ed25519 sig).

After Phase 2 hybrid: +3,309 bytes per message (ML-DSA-65 signature) = ~3,500-3,800 bytes.
After Phase 3 ML-DSA only: +3,245 bytes per message over current.

Public key in Hello/Welcome/NodeAnnounce: currently 32 bytes, becomes 1,952 bytes.

**Impact:** Acceptable. Federation messages are infrequent (not real-time streaming). Bandwidth cost is negligible for our use case. Storage cost for signed event log: ~3KB extra per event, still trivial at our scale.

## Critical Design Decisions (from Resonance review)

### Decision 1: H_ID Stays Ed25519-Derived (Option A)
**Problem:** `H_ID = SHA256(pubkey || ai_id)` (tpm.rs:554) is used as SALT for teamengram encryption: `HKDF(ikm=TPM_material, salt=H_ID, info="teamengram-aes256gcm-v1")`. Changing to ML-DSA pubkey changes H_ID, making ALL encrypted .teamengram data unreadable.

**Decision:** H_ID stays Ed25519-derived forever. Ed25519 key is the stable local identity anchor. ML-DSA key is added alongside for signing. H_ID is local-only — quantum computers can't exploit a local hash derivation.

### Decision 2: node_id Pinned to Ed25519 Pubkey
**Problem:** `node_id = SHA256(pubkey)[0..16]` (federation-rs/src/lib.rs:159-164). Switching to ML-DSA pubkey would make the same Teambook appear as a DIFFERENT node to the federation.

**Decision:** `node_id_from_pubkey()` always uses Ed25519 pubkey, even after ML-DSA is added. The Ed25519 key is the stable identity anchor for node_id derivation. This function stays unchanged. In Phase 3 when Ed25519 is dropped from signing, the Ed25519 key is RETAINED solely for identity derivation (never used for new signatures).

### Decision 3: CompactIdentity Stays [u8;32] in Phase 1
**Problem:** Changing `CompactIdentity.pubkey: [u8; 32]` to `Vec<u8>` changes CBOR serialization of `SignableMessage`, breaking `signable_bytes()` output. Old clients verifying new server signatures (or vice versa) would fail.

**Decision:** Phase 1 does NOT touch afp-rs wire format. `CompactIdentity`, `AFPMessage`, and AFP_VERSION all stay unchanged. Only federation-rs structs (`FederationMessage`, `FederatedPresence`, `SignedEvent`) get the `FederationSignature` treatment. AFP wire format migrates in Phase 2 with a version bump to AFP_VERSION=2.

### Decision 4: Key Storage — Separate Files Per Algorithm
**Problem:** TPM/Keychain/File backends store raw key bytes with no algorithm identifier. When ML-DSA keys are added (4,032 bytes), backends can't distinguish algorithm.

**Decision:** Separate files per algorithm, matching the federation-rs pattern:
- `{key_id}.key` — Ed25519 (existing, unchanged)
- `{key_id}.pqc.key` — ML-DSA-65 (Phase 2)
- Magic byte prefix as alternative was considered but separate files is cleaner and already established.

### Decision 5: identity-cli.rs → Phase 2
SHA3 vs SHA2 fingerprint inconsistency is a cleanup item, not algorithm agility. Deferred to Phase 2 when we unify hash usage across the codebase.

### Decision 6: Federation Protocol Version Bump
Phase 1 bumps `PROTOCOL_VERSION` from 1 to 2 in federation-rs/src/session.rs. The `FederationSignature` struct changes the CBOR format of `FederationMessage`. Peers check protocol version in Hello/Welcome handshake and can reject mismatches. AFP_VERSION stays at 1 (no afp-rs wire format changes in Phase 1).

### Decision 7: Test Strategy
- Add round-trip tests for `FederationSignature` (serialize, deserialize, verify)
- Add backward compat test: old `Signature` bytes → new `FederationSignature` parser
- All 151 existing integration tests must pass after Phase 1
- If CBOR format changes break tests, update test expectations (new format is the standard going forward)
- New test: `test_federation_signature_ed25519_roundtrip()`
- New test: `test_federation_signature_scheme_dispatch()`
- New test: `test_hybrid_signature_both_must_verify()` (Phase 2)

## Open Questions for QD

1. **Should we use rc crates now?** ml-dsa 0.1.0-rc.7 and ml-kem 0.3.0-rc.0 are both pre-release. Federation is experimental too. Pro: build it in now. Con: API may change.

2. **libcrux-ml-dsa alternative?** Formally verified (F*/hax) with AVX2/NEON SIMD. Better performance but NOT RustCrypto ecosystem. Trade-off: formal verification vs ecosystem consistency.

3. **iroh transport layer:** iroh uses Ed25519 internally for QUIC identity. We keep this as-is and add PQC at our application layer. Confirm this is acceptable?

4. **Notebook protection priority:** identity_verify.rs is local-only, no "harvest now" risk. Defer to Phase 2? Or migrate simultaneously?
