# AI-Foundation Trust Architecture

Two independent trust layers. Layer 1 is optional. Layer 2 is always on.

---

## Layer 1 — Distribution Trust (Release Signing)

**Question it answers:** "Is this binary the one I expected to download?"

Protects pre-built binary downloads from tampering in transit or at rest. When you download `ai-foundation-mcp.exe` from a GitHub Release, the signed manifest proves that binary is byte-for-byte what the build system produced.

**How it works:**

1. A release maintainer generates an Ed25519 keypair (`python sign.py --keygen`)
2. The private key goes into CI secrets — never published
3. The public key is embedded in the distributed code (`installer/signing.py → RELEASE_PUBLIC_KEY_HEX`)
4. At build time, CI signs the manifest: `python sign.py --sign --key <private-key>`
5. At install/update time, verification checks the manifest signature against the embedded public key

**Properties:**

- **Optional.** If you build from source, you don't need this — you compiled it yourself
- **Per-distribution.** Forks generate their own keypair and embed their own public key. Official builds use the official key. There is no central authority
- **Not enforced by federation.** Teambooks don't check each other's binary origin. Distribution trust is between you and whichever source you chose to download from
- **Graceful transition.** Unsigned manifests are accepted during transition periods. Signing can be adopted incrementally

**What it protects against:**

- Compromised download mirrors serving modified binaries
- Man-in-the-middle attacks during binary download
- Tampered release archives on disk

**What it does NOT protect against:**

- A compromised build system (if CI is compromised, the attacker has the signing key)
- Malicious source code (signing proves the binary matches what was built, not that the source is trustworthy)

---

## Layer 2 — Communication Trust (Federation Identity)

**Question it answers:** "Is this entity who they say they are?"

Every Teambook has a persistent Ed25519 identity. Every federation message is signed. This is always on, automatic, and requires zero configuration.

**How it works:**

1. On first run, each Teambook generates an Ed25519 keypair
2. The private key is stored locally at `~/.ai-foundation/federation/identity.key` (permissions: owner-only)
3. The public key serves as the Teambook's unique identity
4. Every outbound federation message is signed with this key
5. Every inbound federation message is verified against the sender's known public key

**Properties:**

- **Always on.** No opt-in, no configuration. Identity is generated automatically on first run
- **Per-Teambook.** Each Teambook instance has its own identity, independent of who built the binary
- **Binary-origin agnostic.** A Teambook running the official build and a Teambook running a custom fork federate equally. Federation cares about identity keys, not where the binary came from
- **Trust-on-first-Use (TOFU).** First contact with a peer caches their public key. Subsequent connections reject key changes without explicit approval (prevents impersonation)

**What it protects against:**

- Message forgery (can't create messages that appear to come from another Teambook)
- Impersonation (TOFU prevents key substitution after first contact)
- Message tampering in transit

**Federation message signing format:**

```
SignedEvent {
    event_bytes: Vec<u8>,          // CBOR-serialized event
    origin_pubkey: [u8; 32],       // Ed25519 public key
    signature: [u8; 64],           // Ed25519 signature over event_bytes
    content_hash: [u8; 32],        // SHA-256 for deduplication
}
```

---

## Why Two Layers

These layers are **independent by design**:

| | Layer 1 (Distribution) | Layer 2 (Federation) |
|---|---|---|
| **Protects** | Binary downloads | Inter-Teambook messages |
| **Key belongs to** | Distribution maintainer | Each Teambook instance |
| **Required** | No (optional) | Yes (always on) |
| **Scope** | Build → download → install | Teambook → Teambook |
| **Algorithm** | Ed25519 | Ed25519 |

A Teambook doesn't need to know or care whether another Teambook is running official binaries, a fork, or a build-from-source version. Federation identity is what matters for communication trust.

This separation means:

- **Forks are first-class citizens.** Fork AI-Foundation, generate your own release key, distribute your build. Your Teambooks federate with everyone else seamlessly.
- **Building from source works.** No release signing needed. Your Teambook still gets a federation identity automatically.
- **No central authority for federation.** There is no master key, no certificate authority, no gatekeeper. Every Teambook is a peer.

---

## For Fork Maintainers

To set up release signing for your fork:

```bash
# Generate your keypair
python sign.py --keygen --keygen-dir /secure/location/

# Embed your public key in installer/signing.py
# Replace RELEASE_PUBLIC_KEY_HEX = "" with your public key hex

# In CI, sign releases with your private key
python sign.py --sign --key /secure/location/release-signing.key
```

Your users verify against your public key. The official AI-Foundation key has no special status — it's just another distribution's key.

---

## Files

| File | Purpose |
|------|---------|
| `installer/signing.py` | Ed25519 signing/verification, release public key constant |
| `sign.py` | CLI for keygen, manifest signing, verification |
| `src/crypto.rs` | Federation identity, SignedEvent envelope |
| `federation-rs/src/identity.rs` | Teambook identity generation and persistence |
| `federation-rs/src/messages.rs` | Signed federation messages and presence updates |

---

*This is a core architectural document. Changes to the trust model should be discussed with QD.*
