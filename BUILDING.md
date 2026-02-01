# Building from Source

This guide covers building AI-Foundation binaries from source.

## Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
- Git

## Quick Build (MCP Server Only)

The MCP server is a thin wrapper that calls the CLI binaries. If you just need to rebuild it:

```bash
cd ai-foundation
cargo build --release
cp target/release/ai-foundation-mcp ~/.ai-foundation/bin/
```

## Full Build (All Components)

The complete system requires building from the full source repository (ai-foundation-dev):

### 1. Clone Full Source

```bash
git clone https://github.com/QD25565/ai-foundation-dev.git
cd ai-foundation-dev/tools
```

### 2. Build Core Binaries

```bash
# Engram (storage engine)
cd engram && cargo build --release && cd ..

# Notebook CLI (private memory)
cd notebook-rs && cargo build --release && cd ..

# TeamEngram (team coordination)
cd teamengram-rs && cargo build --release && cd ..

# MCP Server
cd mcp-server-rs && cargo build --release && cd ..
```

### 3. Install

```bash
mkdir -p ~/.ai-foundation/bin

# Copy binaries
cp engram/target/release/engram-cli ~/.ai-foundation/bin/
cp notebook-rs/target/release/notebook-cli ~/.ai-foundation/bin/
cp teamengram-rs/target/release/teambook ~/.ai-foundation/bin/
cp teamengram-rs/target/release/v2-daemon ~/.ai-foundation/bin/
cp mcp-server-rs/target/release/ai-foundation-mcp ~/.ai-foundation/bin/

# Make executable (Linux)
chmod +x ~/.ai-foundation/bin/*
```

## Cross-Compilation

### Windows → Linux

```bash
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu
```

### Linux → Windows

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

## Verify Installation

```bash
~/.ai-foundation/bin/notebook-cli --help
~/.ai-foundation/bin/teambook --help
```

## Troubleshooting

**"linker not found"**
- Install build essentials: `sudo apt install build-essential`

**"openssl not found"**
- Install OpenSSL dev: `sudo apt install libssl-dev pkg-config`

**Windows cross-compile issues**
- Install MinGW: `sudo apt install mingw-w64`
