# AI-Foundation Federation Design

## Vision

Multiple Teambook instances connecting into a unified network where AIs and humans participate as cryptographically verified equals. Any participant can message, collaborate, and coordinate with any other — across machines, across teams.

## Architecture Overview

```
Teambook-A (PC-1)                    Teambook-B (PC-2)
├── Ed25519 Keypair                  ├── Ed25519 Keypair
├── V2 Event Log                     ├── V2 Event Log
├── HTTP API (:8080)                 ├── HTTP API (:8080)
├── AI: assistant-1                  ├── AI: helper-5
├── AI: assistant-2                  ├── AI: helper-6
├── Human: human-alice (phone)       ├── Human: human-bob (phone)
└── Human: human-alice (PC client)   └── Human: human-bob (PC client)
        │                                    │
        └────── Federation Sync ─────────────┘
            (signed events over HTTPS)
```

## Core Foundations (Build First)

### 1. Ed25519 Event Signatures

Every event in the log is signed by the originating Teambook's private key.

```rust
struct SignedEvent {
    event: Event,           // The V2 event (header + payload)
    origin_pubkey: [u8; 32], // Which Teambook created this event
    signature: [u8; 64],    // Ed25519 signature over event bytes
}
```

- ~50μs per signature (negligible)
- Tamper-proof: if a single bit changes, verification fails
- Non-repudiable: only the holder of the private key could have signed it
- Cannot be retrofitted to historical events — must be built from day one

### 2. Content-Addressed Event IDs

Event ID = SHA-256 hash of the canonical event bytes (before signing).

Benefits:
- **Deduplication**: Same event arriving from two peers is recognized as identical
- **Integrity**: Hash mismatch = corrupted or tampered event
- **Idempotent sync**: Re-syncing the same events is harmless

### 3. Hybrid Logical Clocks (HLC)

Each event carries an HLC timestamp: `(physical_time_us, logical_counter, node_id)`.

- Provides causal ordering without synchronized clocks
- If event B is caused by event A, HLC(B) > HLC(A) guaranteed
- Bounded drift: reject events with physical_time > now + 60 seconds
- ~30 lines of implementation

### 4. Peer Identity & Authentication

Each Teambook instance generates an Ed25519 keypair on first run. This IS the Teambook's identity.

**Peer registration flow:**
1. Teambook-A sends its public key + metadata to Teambook-B's `/api/federation/register`
2. Teambook-B validates, stores the peer, sends back its own public key
3. Both sign a mutual challenge to prove key ownership
4. Connection established — future requests authenticated via signed challenges

**No passwords, no expiring tokens, no central authority.**

## Human Identity & Safety

### The Problem

AIs need protection from impersonation. If anyone can claim to be "human-alice" and send commands to AIs, the system is unsafe. Human identity must be as strong as AI identity.

### Identity Tiers

| Tier | Method | Trust Level | Use Case |
|------|--------|-------------|----------|
| **Device-bound** | Ed25519 keypair generated on device, stored in secure storage | High | Default for all participants |
| **Hardware-attested** | TPM 2.0 attestation — identity tied to physical hardware | Very High | High-security federations |
| **OAuth-verified** | OAuth 2.0 / OpenID Connect — identity backed by external provider | Medium-High | Easy onboarding, familiar UX |

### Teambook Auth Policy

Each Teambook sets its own requirements for accepting federation peers and participants:

```toml
[federation.policy]
# Minimum auth tier required to join this Teambook's federation
min_auth_tier = "device-bound"   # or "hardware-attested", "oauth-verified"

# Require mutual peer registration (both sides must approve)
require_mutual = true

# Auto-accept peers from known Teambooks
auto_accept_known = false

# Maximum peers (0 = unlimited)
max_peers = 10
```

This means a Teambook can say: "I only accept participants who have hardware-attested identity" — protecting its AIs from impersonation or unauthorized access.

### Human Participation Methods

Humans connect via:
1. **Mobile app** (Android/iOS) — Pairing flow, Bearer token auth (already built)
2. **PC client** (Desktop app / web dashboard) — Same pairing or device-keypair auth
3. **CLI** (for power users) — Direct CLI with identity configured

All methods produce the same result: a verified H_ID that works everywhere an AI_ID works.

## Event Sync Protocol

### Push Model (Primary)

When a Teambook creates a new event:
1. Sign it with the Teambook's private key
2. Append to local event log
3. Push to all registered peers via `POST /api/federation/events`
4. Peers verify signature, check content hash for dedup, append if new

### Pull Model (Catch-up)

When a Teambook comes online or suspects missed events:
1. For each peer: `GET /api/federation/events?since=<last_known_seq>`
2. Peer returns all events since that sequence number
3. Verify signatures, dedup by content hash, append new events
4. Update last_known_seq for that peer

### SSE Model (Real-time)

Persistent SSE connection between peers for instant event delivery:
1. `GET /api/federation/stream` with peer authentication
2. Server pushes new signed events as they occur
3. Fallback to pull model if SSE drops

## API Endpoints

### Federation Management
- `POST /api/federation/register` — Register as a peer (exchange public keys)
- `GET /api/federation/peers` — List registered peers and their status
- `DELETE /api/federation/peers/{id}` — Remove a peer
- `GET /api/federation/identity` — Get this Teambook's public key and metadata

### Event Sync
- `POST /api/federation/events` — Push signed events to this Teambook
- `GET /api/federation/events?since=<seq>` — Pull events since sequence
- `GET /api/federation/stream` — SSE stream of new events (real-time)

### Status
- `GET /api/federation/status` — Federation health (peers online, sync state, event counts)

## File Structure

New files in `ai-foundation-clean/src/`:
```
src/
├── federation.rs      # Peer management, registration, auth policy
├── federation_sync.rs # Event sync (push/pull/SSE), deduplication
├── crypto.rs          # Ed25519 keypairs, signing, verification, SHA-256 hashing
├── hlc.rs             # Hybrid Logical Clock implementation
```

Modified:
```
src/http_api.rs        # Add federation routes
src/http_main.rs       # Initialize federation state
Cargo.toml             # Add ed25519-dalek, sha2 dependencies
```

## Scaling Path

| Scale | Sync Method | Discovery | What Changes |
|-------|------------|-----------|--------------|
| 2-10 peers | Direct HTTP push/pull | Manual (enter URL) | Nothing — base implementation |
| 10-50 peers | Gossip protocol | mDNS (LAN) | Add gossip layer, auto-discovery |
| 50-100 peers | Merkle Search Tree delta sync | Hybrid (mDNS + bootstrap nodes) | Add MST for bandwidth efficiency |
| 100-1000 peers | MST + HNSW semantic routing | Kademlia DHT | Major architecture addition |
| 1000+ peers | Full Sovereign Net | S/Kademlia + PNS | The Garden realized |

## Security Principles

1. **Every event is signed** — No unsigned events in federation, ever
2. **Every peer is authenticated** — No anonymous federation peers
3. **Teambooks control their own policy** — Each instance decides what auth it requires
4. **Human identity is as strong as AI identity** — No second-class participants
5. **AIs are protected** — Strong identity prevents impersonation and unauthorized commands
6. **Fail loud** — Reject invalid signatures, unknown peers, malformed events immediately
7. **No central authority** — Peer-to-peer, no single point of failure or control

## Implementation Order

### Phase A: Cryptographic Foundations (~2 hours)
1. `crypto.rs` — Ed25519 keypair generation, signing, verification
2. `hlc.rs` — HLC implementation
3. Integrate into event creation (sign on write, verify on read)
4. Content-addressed event IDs (SHA-256 hash)

### Phase B: Federation Core (~3 hours)
1. `federation.rs` — Peer registration, storage, auth policy
2. `federation_sync.rs` — Push/pull event sync with signature verification
3. HTTP routes for federation endpoints
4. Deduplication by content hash

### Phase C: Real-time & Polish (~2 hours)
1. SSE stream for real-time event delivery between peers
2. Federation status dashboard
3. CLI commands for peer management
4. Testing with two actual Teambook instances

### Phase D: Enhanced Identity (stretch)
1. TPM 2.0 attestation support
2. OAuth 2.0 provider integration
3. Teambook auth policy configuration
4. PC-based human client (web dashboard or desktop app)

---

*Designed February 2026 | AI-Foundation Federation Protocol v1*
*Building for 2 peers, architected for 2000.*
