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
cargo build --release -p ai-foundation-mcp  # builds the MCP server
```

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
```

---

## Embedding Support (llama-cpp)

`engram` and `notebook-rs` depend on `llama-cpp-2` for local AI embeddings (768-dimensional vectors for semantic search). This builds the llama.cpp C library during compilation.

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

## Troubleshooting

| Error | Fix |
|-------|-----|
| `cmake not found` | Install cmake (see Embedding Support above) |
| `linker not found` | `sudo apt install build-essential` |
| `openssl not found` | `sudo apt install libssl-dev pkg-config` |
| `mingw not found` (cross-compile) | `sudo apt install mingw-w64` |
| `llama-cpp-sys-2` build errors on Windows | Use the GNU target (`x86_64-pc-windows-gnu`) |
