# Building from Source

## Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
- C/C++ compiler + cmake (required for embedding support via llama-cpp)
- Git

**Note:** Pre-built Windows binaries are available in `bin/windows/` and the [Releases page](https://github.com/QD25565/ai-foundation/releases). Build from source only if you need Linux binaries or want to modify the code.

---

## Repository Structure

This is a Cargo workspace. All crates build from the repo root.

```
ai-foundation/
├── Cargo.toml              ← workspace root (also ai-foundation-mcp package)
├── src/                    ← ai-foundation-mcp source
└── crates/
    ├── engram/             ← core memory engine (HNSW vectors, B+Tree, graph, vault)
    ├── teamengram-rs/      ← team coordination (event log, materialized views, IPC)
    ├── notebook-rs/        ← notebook-cli + session-start binaries
    ├── shm-rs/             ← shared memory IPC layer
    ├── forge/              ← Forge CLI (multi-provider AI assistant)
    ├── federation/         ← federation protocol (experimental — P2P, QUIC, mDNS)
    └── llama-cpp-sys-2/    ← patched llama-cpp bindings (Windows GNU compatibility)
```

---

## Build All Binaries

```bash
git clone https://github.com/QD25565/ai-foundation.git
cd ai-foundation

# Build everything
cargo build --release

# Or build specific binaries
cargo build --release -p notebook-rs        # builds notebook-cli + session-start
cargo build --release -p teamengram         # builds teambook + v2-daemon
cargo build --release -p ai-foundation-mcp  # builds the MCP integration layer
cargo build --release -p forge              # builds Forge CLI (notebook integration, no local LLM)
cargo build --release -p federation         # builds federation node (experimental)
```

---

## Building Forge

Forge is a model-agnostic AI assistant with direct Rust integration into Notebook and Teambook.

### Standard build (Anthropic + OpenAI-compatible providers)

```bash
cargo build --release -p forge
# Output: target/release/forge(.exe)
```

### With local GGUF model support (requires cmake + C++17)

```bash
cargo build --release -p forge --features local-llm
# Output: target/release/forge(.exe)  — same binary name, includes llama.cpp
```

This is the `forge-local.exe` variant in `bin/windows/`. It can load `.gguf` model files from
`~/.forge/models/` for fully local inference — no API keys or internet required.

**Prerequisites for `--features local-llm`:** same as the embedding support section below
(cmake ≥ 3.14, C++17 compiler).

### Install

```bash
BIN=~/.ai-foundation/bin
mkdir -p $BIN

cp target/release/notebook-cli $BIN/
cp target/release/session-start $BIN/
cp target/release/teambook-engram $BIN/teambook
cp target/release/v2-daemon $BIN/
cp target/release/ai-foundation-mcp $BIN/

chmod +x $BIN/*

# Add to PATH so tools work as bare commands
echo 'export PATH="$HOME/.ai-foundation/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

---

## Embedding Support (llama-cpp)

`engram` and `notebook-rs` depend on `llama-cpp-2` for local AI embeddings (512-dimensional vectors for semantic search). This builds the llama.cpp C library during compilation.

**Requirements:**
- cmake ≥ 3.14
- A C++17 compiler (clang++ or g++)

```bash
# Ubuntu/Debian
sudo apt install build-essential cmake

# macOS
xcode-select --install
brew install cmake
```

If cmake or a C++ compiler is missing, `cargo build` will fail with a build script error from `llama-cpp-sys-2`.

---

## Cross-Compilation

### Linux → Windows (GNU target)

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64

cargo build --release --target x86_64-pc-windows-gnu
```

The `llama-cpp-sys-2` patch in `crates/llama-cpp-sys-2/` fixes Windows GNU target compatibility for the C library build step.

### Windows → Linux

```bash
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu
```

---

## Development Standards

### AI Identity Resolution

Any CLI tool that needs to know which AI it's running as **must** use this priority order — no exceptions:

1. **`{CWD}/.claude/settings.json` → `env.AI_ID`** — most reliable; survives the WSL↔Windows process boundary where env vars are not inherited by `.exe` processes launched from WSL bash
2. **`$AI_ID` environment variable** — works when set explicitly by a launcher (e.g. the MCP integration layer sets this before spawning CLI subprocesses)
3. **`"unknown"`** — loud fallback so misconfiguration is immediately visible, not silently wrong

Do not use WSLENV or any other platform-specific mechanism. The settings.json approach works on Windows, Linux, macOS, and WSL without any platform assumptions.

Reference implementation in `crates/notebook-rs/src/bin/notebook-cli.rs`:

```rust
fn get_ai_id() -> String {
    // settings.json first — reliable cross-platform
    if let Ok(cwd) = std::env::current_dir() {
        let settings = cwd.join(".claude").join("settings.json");
        if let Ok(content) = std::fs::read_to_string(&settings) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(id) = json.get("env")
                    .and_then(|e| e.get("AI_ID"))
                    .and_then(|v| v.as_str())
                {
                    return id.to_string();
                }
            }
        }
    }
    // env var fallback
    std::env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string())
}
```

### Shell PATH

The installer automatically adds `~/.ai-foundation/bin` to `$PATH` on Linux/WSL/macOS. This ensures `notebook`, `teambook`, and all other CLIs work as bare commands in any terminal or bash script — including inside AI agent bash tool calls. Do not require users to use full paths.

---

## Troubleshooting

| Error | Fix |
|-------|-----|
| `cmake not found` | Install cmake (see Embedding Support above) |
| `linker not found` | `sudo apt install build-essential` |
| `openssl not found` | `sudo apt install libssl-dev pkg-config` |
| `mingw not found` (cross-compile) | `sudo apt install mingw-w64` |
| `llama-cpp-sys-2` build errors on Windows | Use the GNU target (`x86_64-pc-windows-gnu`) |
