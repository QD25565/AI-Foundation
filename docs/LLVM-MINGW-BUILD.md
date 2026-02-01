# Building notebook-cli with Embeddings on Windows

## The Problem

Building `notebook-cli` with the `llama-cpp-2` crate on Windows fails with MSVC due to CRT (C Runtime) mismatch errors:

```
error LNK2019: unresolved external symbol fread
error LNK2019: unresolved external symbol fwrite
error LNK2019: unresolved external symbol fseek
...
```

This happens because llama.cpp's C runtime is incompatible with Rust's MSVC target.

## The Solution: LLVM-MinGW

Use the LLVM-MinGW UCRT toolchain instead of MSVC. This provides a compatible C runtime.

### Prerequisites

1. **Install LLVM-MinGW** (via winget):
   ```powershell
   winget install MartinStorsjo.LLVM-MinGW.UCRT
   ```

2. **Add Rust GNU target**:
   ```bash
   rustup target add x86_64-pc-windows-gnu
   ```

3. **Verify installation**:
   ```bash
   where x86_64-w64-mingw32-gcc
   rustup target list --installed | grep gnu
   ```

### Building

From the `tools/notebook-rs` directory:

```bash
RUSTFLAGS="-C link-arg=-lc++ -C link-arg=-lunwind" cargo build --release --target x86_64-pc-windows-gnu --features embeddings
```

**Explanation:**
- `RUSTFLAGS="-C link-arg=-lc++ -C link-arg=-lunwind"` - Links the C++ runtime libraries
- `--target x86_64-pc-windows-gnu` - Uses MinGW instead of MSVC
- `--features embeddings` - Enables the llama-cpp-2 dependency

### Required DLLs

The built binary requires these DLLs from LLVM-MinGW (copy to `bin/` alongside the executable):

| DLL | Purpose |
|-----|---------|
| `libc++.dll` | C++ standard library |
| `libunwind.dll` | Stack unwinding |
| `libwinpthread-1.dll` | POSIX threads |

**Location:** `<LLVM-MinGW-install>/x86_64-w64-mingw32/bin/`

Example:
```bash
LLVM_BIN="C:/Users/.../LLVM-MinGW.../x86_64-w64-mingw32/bin"
cp "$LLVM_BIN/libc++.dll" "$LLVM_BIN/libunwind.dll" "$LLVM_BIN/libwinpthread-1.dll" bin/
```

### The Patch

The `patches/llama-cpp-sys-2/` directory contains a patched `build.rs` that:

1. **Detects GNU target** and uses `.a` files instead of `.lib`:
   ```rust
   let target = std::env::var("TARGET").unwrap_or_default();
   let is_gnu = target.contains("gnu");
   let lib_pattern = if cfg!(windows) && !is_gnu {
       "*.lib"
   } else if cfg!(windows) && is_gnu {
       "*.a"  // <-- Key fix
   } ...
   ```

2. The patch is applied via `Cargo.toml`:
   ```toml
   [patch.crates-io]
   llama-cpp-sys-2 = { path = "patches/llama-cpp-sys-2" }
   ```

### Build Options

| Build | Command | Size | Features |
|-------|---------|------|----------|
| **Without embeddings** (fast) | `cargo build --release --no-default-features` | ~3.5MB | Basic notebook, no semantic search |
| **With embeddings** (full) | See above with RUSTFLAGS | ~8.5MB | Full semantic search, embeddings |

### Model File

For embeddings, place the model in `bin/`:
- **File:** `embeddinggemma-300M-Q8_0.gguf`
- **Size:** ~314MB
- **Dimensions:** 768
- **Source:** [ggml-org/embeddinggemma-300M-GGUF](https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF)

### Performance

| Metric | Rust (llama.cpp) | Python (PyTorch) |
|--------|------------------|------------------|
| Embedding time | 650ms | 10,788ms |
| Speedup | **16.6x faster** | baseline |
| Binary size | ~8.5MB | 2GB+ |
| Footprint ratio | **~200x smaller** | baseline |

### Troubleshooting

**Error: "cannot find -lc++"**
- Ensure LLVM-MinGW `bin/` is in PATH
- Or use full path in RUSTFLAGS

**Error: "*.lib not found"**
- The patch isn't being applied
- Check `Cargo.toml` has the `[patch.crates-io]` section

**Runtime: "DLL not found"**
- Copy the 3 required DLLs to same folder as executable

---

*Last updated: 2025-12-06*
