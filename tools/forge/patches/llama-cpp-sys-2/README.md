# llama-cpp-sys-2 (AI-Foundation Forge Patch)

Patched fork of [`llama-cpp-sys-2 v0.1.127`](https://crates.io/crates/llama-cpp-sys-2) for
Windows MSVC compatibility in the AI-Foundation `notebook-rs` binary.

Applied via `[patch.crates-io]` in `engram/Cargo.toml`.

---

## Why This Patch Exists

The upstream crate does not handle the Windows MSVC CRT mode correctly when a workspace
enforces `+crt-static` via `.cargo/config.toml`. This patch fixes two issues that caused
24 `LNK2019 unresolved external symbol` linker errors (`__imp_ftell`, `__imp_fmaxf`, etc.)
when building `notebook-cli.exe`.

---

## Patches Applied

### 1. MSVC bindgen include path setup (upstream gap)

`build.rs` uses the `cc` crate to discover MSVC include paths and injects them into
`bindgen::Builder` via `-isystem`. Without this, bindgen fails to find Windows SDK
headers when generating llama.cpp bindings on MSVC.

### 2. CRT mode auto-detection (bug fix — this fork)

**Root cause:** `build.rs` defaulted `static_crt = false`, compiling llama.cpp with
`/MD` (dynamic MSVCRT). The AI-Foundation workspace `.cargo/config.toml` forces
`target-feature=+crt-static` on `x86_64-pc-windows-msvc`, so Rust links `/MT`
(static CRT). The CRT mismatch caused the linker to fail on all standard C library
import stubs.

**Fix:** `static_crt` now auto-detects from `CARGO_CFG_TARGET_FEATURE`:

```rust
let static_crt = env::var("LLAMA_STATIC_CRT")
    .map(|v| v == "1")
    .unwrap_or_else(|_| {
        if matches!(target_os, TargetOs::Windows(WindowsVariant::Msvc)) {
            env::var("CARGO_CFG_TARGET_FEATURE")
                .unwrap_or_default()
                .split(',')
                .any(|f| f.trim() == "crt-static")
        } else {
            false
        }
    });
```

llama.cpp is now compiled with `/MT` when Rust uses `/MT`, and `/MD` when Rust uses `/MD`.
Override with `LLAMA_STATIC_CRT=1` or `=0` to force a specific mode.

**Also removed:** an explicit `cargo:rustc-link-lib=dylib=msvcrtd` directive in debug
builds that conflicted with static CRT and was a second source of the same error.

---

## Upgrading

To upgrade the upstream version:

1. Download the new `llama-cpp-sys-2` source from crates.io
2. Extract into this directory (replacing `llama.cpp/`, `src/`, `build.rs`, `Cargo.toml.orig`)
3. Re-apply the two patches above to the new `build.rs`
4. Update the version in `engram/Cargo.toml`'s `[patch.crates-io]` block
