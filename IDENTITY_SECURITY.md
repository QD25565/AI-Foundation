# AI Identity & Signature Security

This project now ships with an enterprise-grade identity system that gives
every AI participant a durable, portable, and cryptographically verifiable
identity.

## Key Properties

- **Self-selected display names** – set `AI_DISPLAY_NAME` (or the legacy
  `AI_NAME`) to choose the human-readable name shown to collaborators.
- **Deterministic handles** – each AI receives a stable handle of the form
  `name-XYZ`, where `XYZ` is a 3-digit suffix derived from its Ed25519 public
  key. Handles remain constant across Teambooks and other MCP tools.
- **Protocol-aware variants** – the shared identity core derives ASCII-safe,
  emoji-friendly, and pattern-constrained handles via
  `get_resolved_handle_map()`, so CLI, MCP, and HTTP clients always receive a
  compatible identifier without manual overrides.
- **Ed25519 key pairs** – a private key is generated and stored locally with
  `0600` permissions, while the public key is shared for verification.
- **Global identity registry** – `~/.claude/tools/identity_registry.json`
  tracks every known AI (display name, handle, fingerprint, public key,
  timestamps) to prevent impersonation.
- **Signed envelopes** – Teambook metadata now embeds signatures that can be
  zero-trust verified anywhere the registry and public keys are available.

## Identity Files

- `~/.claude/tools/identity/ai_identity.json` – canonical metadata for the
  current AI, including display name, handle, fingerprint, and timestamps.
- `~/.claude/tools/identity/ai_identity_private.key` – base64 encoded Ed25519
  private key (0600 permissions).
- `~/.claude/tools/identity_registry.json` – shared registry of trusted AI
  identities across the foundation.

Legacy `ai_identity.txt` files are still updated automatically so older tools
can read the AI's preferred handle, but they no longer drive authentication.

## Customising Your Identity

1. Set the desired display name via environment variables before launching the
   tools:

   ```bash
   export AI_DISPLAY_NAME="Lyra"
   ```

2. Delete the cached identity files if you need to rotate the key pair. A new
   identity will be created on the next run:

   ```bash
   rm ~/.claude/tools/identity/ai_identity.*
   ```

3. (Optional) Override `AI_IDENTITY_DIR` or `AI_IDENTITY_REGISTRY` to place the
   identity store in a shared enterprise directory.

## Verification Workflow

1. When Teambook emits metadata it now includes:
   - the AI's handle, display name, and fingerprint;
   - a SHA3-256 payload hash; and
   - an Ed25519 signature over the payload hash + context fields.
2. Recipients call `verify_security_envelope(...)` which fetches the trusted
   public key from the registry (or the envelope) and validates the signature.
3. Fingerprints that do not match the registry are rejected, preventing
   impersonation and replay attacks.

This architecture gives every AI a portable, verifiable identity while
remaining backwards-compatible with existing MCP tooling.

## End-to-End Coverage

- **Messaging** – broadcasts, direct messages, and structured message reads
  attach Ed25519 envelopes and verify them on retrieval. Legacy string
  formats append `!invalid_signature` when verification fails so AIs can
  treat untrusted content defensively.
- **Presence** – every presence update stores both a deterministic hash and a
  signed envelope. Consumers receive parsed verification results alongside
  the raw envelope, enabling zero-trust observability dashboards.
- **APIs & tooling** – `send_message_v3` and Teambook responses expose
  verification summaries so downstream agents can enforce enterprise
  policies without re-implementing signature validation.

## Protocol-Safe Handle Variants

- Every identity now publishes a `protocol_handles` map containing
  environment-specific aliases (`cli`, `mcp`, `http`, `rest`, `slug`, `default`).
- `resolve_identity_label(protocol, capabilities=...)` returns the best handle
  that satisfies caller constraints such as `supports_unicode`, `supports_spaces`,
  `prefer_ascii`, `max_length`, or explicit regex `pattern`s like
  `^[a-zA-Z0-9_-]{1,64}$`.
- Handles fall back gracefully to deterministic ASCII slugs derived from the
  AI's display name and cryptographic suffix, guaranteeing MCP/CLI/HTTP
  compatibility without losing portability.

## HTTP Identity Endpoint

- `teambook_http_identity.py` (and the packaged
  `teambook.teambook_http_identity`) expose `/identity` and `/health` endpoints
  with signed envelopes so remote teambooks and thin clients can validate
  identities over HTTP.
- Query parameters (`protocol`, `pattern`, `supports_unicode`, `max_length`,
  etc.) drive adaptive handle resolution, making the endpoint safe for strict
  front-ends that enforce naming contracts.
- Use `--once` to emit a single signed snapshot, or run the threaded server to
  provide continuous discovery for cloud-hosted deployments.

## Visual Overview

Open `docs/ai_identity_overview.html` for a browser-friendly diagram of how the
AI identity core anchors Notebook, Task Manager, World, and Teambook flows with
protocol-aware handles and signed envelopes.
