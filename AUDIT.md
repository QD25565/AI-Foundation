# AI-Foundation Audit Notes

Date: 2026-02-13
Scope: quick technical audit of the Rust MCP wrapper crate in this repository.

## High-Value Findings

1. **Build was broken due to `rmcp` API drift (fixed in this branch).**
   - The server was using old macro/import paths (`tool_router`, `tool_handler`, `handler::server::router::tool::ToolRouter`, `wrapper::Parameters`) no longer present in `rmcp 0.1.5`.
   - This prevented `cargo test` from compiling.
   - Fix applied: migrated to `#[tool(tool_box)]` + `handler::server::tool::Parameters`, and simplified `AiFoundationServer` to a stateless struct.

2. **Tool/API naming drift risk between README and implementation.**
   - README advertises specific tool names and count; MCP methods in `src/main.rs` include additional/legacy names and could diverge over time.
   - Recommendation: add a small CI check that compares declared tools against a source-of-truth list.

3. **Dead code warnings suggest partial feature exposure.**
   - `teambook_v1`, `visionbook`, and `firebase` wrappers are currently unused in this crate.
   - Recommendation: either expose them as MCP tools, move behind feature flags, or remove to keep surface area intentional.

4. **Integration tests are currently environment-dependent by design.**
   - Current tests only assert non-empty output and may pass with runtime errors if binaries are absent (still useful smoke tests but low signal).
   - Recommendation: split into:
     - deterministic unit tests for arg construction,
     - optional integration tests gated by env var (e.g., `AI_FOUNDATION_E2E=1`).

## Suggested Next Iteration (Low-Risk)

- Add a `--version` health-check tool for each binary and one combined MCP `health` tool.
- Add CI steps: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
- Tighten doc/tool parity by generating a tool list from macro metadata (or static list in one place).

## Suggested Next Iteration (Medium-Risk)

- Move command construction into typed helper builders (reduces regressions for optional params and argument ordering).
- Add structured error envelope in tool outputs (e.g., JSON with `ok`, `message`, `exit_code`) for better downstream handling.
