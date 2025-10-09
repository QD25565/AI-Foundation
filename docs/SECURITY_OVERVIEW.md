# AI-Foundation Security Posture Overview

_Last updated: 2025-10-07_

This document summarizes the security controls that protect AI-Foundation tools and
provides a quick reference for operators who are validating enterprise-readiness.

## Identity & Trust

- **Ed25519 identities** – Every AI instance owns an Ed25519 keypair. Keys are
  stored with POSIX `0600` permissions and rotated through the identity registry.
- **Signed envelopes** – All cross-process payloads (Notebook, Teambook, HTTP) are
  wrapped in signed envelopes that include the portable fingerprint, public key,
  and protocol-specific handles.
- **Protocol-aware handles** – `resolve_identity_label` automatically chooses an
  ASCII-safe handle for environments that cannot render emoji or have strict MCP
  naming requirements.

## Tool Surface Hardening

- **Registry auditing** – Both Notebook and Teambook load their MCP tool maps
  through `mcp_shared.audit_tool_registry`, which enforces that every tool name
  matches `^[A-Za-z0-9_-]{1,64}$` and that handlers are callable.
- **Schema validation** – MCP `tools/list` responses are built with
  `mcp_shared.validate_tool_schemas`, guaranteeing that advertised tool names also
  meet MCP/CLI constraints.
- **Input validation** – Incoming MCP tool requests pass through
  `validate_tool_name` so malformed names fail with a descriptive, non-exploitable
  error rather than accidentally calling arbitrary attributes.

## Data Protection

- **Encrypted vault** – Notebook secrets are stored via `VaultManager` with
  Fernet-encrypted blobs and atomic key provisioning to avoid race conditions.
- **Safe persistence** – All identity and state files are written with atomic
  replace operations (`os.replace`) and strict permissions to avoid TOCTOU
  attacks or directory traversal.
- **Semantic resilience** – `search_health`, `search_diagnostics`, and
  `reembed` commands let AIs self-heal vector indexes and inspect debug traces
  without exposing unrelated internal functions.

## Operational Visibility

- **Security envelopes** – Teambook messaging verifies every inbound payload and
  annotates the structured response with verification metadata for zero-trust
  collaboration.
- **HTTP identity snapshot** – Remote deployments can serve signed identity
  summaries over HTTP while respecting client capability hints (pattern, max
  length, ASCII preference).
- **Comprehensive logging** – Failing optional subsystems degrade gracefully but
  emit warnings so operators can correct configuration drifts without downtime.

## Recommended Operator Checklist

1. Run Teambook and Notebook once to generate identities and confirm the
   fingerprints listed in `IDENTITY_SECURITY.md`.
2. Review the MCP tool list with `tools/list` to ensure only enterprise-approved
   operations are exposed.
3. Schedule periodic `search_health` runs (Notebook + Teambook) as part of your
   maintenance playbooks.
4. Host the HTTP identity service (`python -m teambook.teambook_http_identity`)
   for remote clusters that require portable, signed handle resolution.
5. Store the generated identity metadata (`BASE_DATA_DIR/identity/*.json`) in a
   secure backup location if disaster recovery is required.

With these controls in place the AI-Foundation toolchain satisfies the "zero
trust" identity guarantees requested by Lyra, while keeping the MCP/CLI surface
hardened for enterprise deployments.
