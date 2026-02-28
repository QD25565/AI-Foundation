# AI-Foundation Audit Notes

## v58 Audit — 2026-03-01

Scope: Full codebase audit covering security, data integrity, and quality.

### Security

- **Encryption at rest:** AES-256-GCM for .teamengram event log payloads. Deterministic nonces from sequence numbers. Backward-compatible (FLAG_ENCRYPTED detection).
- **Key storage:** DPAPI software encryption with TPM presence verification (Windows). Ed25519 identity keys per AI. Volatile key zeroization via `zeroize` crate.
- **Silent error suppression:** Audited and fixed across v2_client.rs (7 fixes), event_log.rs (2), outbox.rs (5), teamengram-daemon.rs (4), shadow.rs (1), page.rs (1). All critical paths now fail loudly.
- **Unsafe memory:** page.rs leaf/branch entry counts clamped to MAX values. outbox.rs `from_raw_parts` capacity validated against mmap length.
- **Zero polling:** Full codebase audit. All sleep/interval patterns replaced with OS-native wait primitives (Named Events on Windows, POSIX semaphores on Linux).

### Data Integrity

- **B+Tree checksums:** CRC32 verification on both read (catch disk corruption) and commit (catch in-memory corruption). Every modified page verified before flush.
- **Event log compaction:** Retention-based policy, preserves encrypted payloads as-is during copy.
- **V1 to V2 migration:** Automated, checksum-correct after branch entry sorting.

### Test Coverage

- **Integration tests:** 151 pass, 0 fail, 2 ignored. 17 test suites covering all 45 active event types (3 deprecated types excluded).
- **MCP conformance:** 8 tests validating JSON-RPC 2.0 stdio protocol, tool invocation, and error handling.
- **Unit tests:** 101 teamengram-rs lib, 167 engram, 161 federation, 25 afp-rs.

### Open Items

- **Federation stream semaphore:** Global instead of per-peer. Under load, one slow peer could block others. Medium severity, not blocking for LAN deployment.
- **KDF strength:** afp-rs file fallback uses bare SHA-256 for key derivation. Should be Argon2id or PBKDF2. Mitigated by non-user-chosen input (AI_ID). Medium severity.
- **Page nonce truncation:** crypto.rs page_nonce truncates txn_id to 32 bits. Nonce collision after 2^32 transactions. Low severity at current scale.
- **Forge local model:** No generation model bundled. Planned: purpose-built fine-tuned model (see AI-Foundation Daemon).

### Previous Audit (v57, 2026-02-13)

All findings from the v57 audit have been addressed:
1. Build breakage from `rmcp` API drift — resolved, builds clean with 0 warnings.
2. Tool/API naming drift — tool count now audited from source (28 tools in MCP server).
3. Dead code (deprecated modules, teambook_v1 wrappers) — removed in v58 source sync.
4. Integration test quality — replaced with 151 targeted integration tests using TestHarness (isolated dual-daemon spawning).
