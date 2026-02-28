#!/usr/bin/env bash
# AI-Foundation — One-line installer for Linux and macOS
#
# Usage:
#   curl -fsSL https://github.com/QD25565/ai-foundation/raw/main/install.sh | bash
#   wget -qO- https://github.com/QD25565/ai-foundation/raw/main/install.sh | bash
#
# Options (via environment variables):
#   VERSION=57 ./install.sh     # Install a specific version
#   BIN_DIR=~/custom ./install.sh  # Custom install directory
#   SKIP_PATH=1 ./install.sh    # Skip PATH setup

set -euo pipefail

# ── Configuration ─────────────────────────────────────────
REPO="QD25565/ai-foundation"
GITHUB="https://github.com/${REPO}"
API="https://api.github.com/repos/${REPO}"
INSTALL_DIR="${BIN_DIR:-${HOME}/.ai-foundation/bin}"

# ── Formatting ────────────────────────────────────────────
BOLD='\033[1m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
CYAN='\033[0;36m'
RESET='\033[0m'

ok()   { printf "${GREEN}  ✓ %s${RESET}\n" "$1"; }
info() { printf "${CYAN}  → %s${RESET}\n" "$1"; }
warn() { printf "${YELLOW}  ⚠ %s${RESET}\n" "$1"; }
err()  { printf "${RED}  ✗ %s${RESET}\n" "$1"; }
die()  { err "$1"; exit 1; }

banner() {
    printf "\n${BOLD}"
    printf "     █████╗ ██╗\n"
    printf "    ██╔══██╗██║\n"
    printf "    ███████║██║\n"
    printf "    ██╔══██║██║\n"
    printf "    ██║  ██║██║\n"
    printf "    ╚═╝  ╚═╝╚═╝\n"
    printf "\n    F O U N D A T I O N${RESET}\n\n"
}

# ── Platform Detection ────────────────────────────────────
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="macos" ;;
        *)       die "Unsupported OS: $(uname -s). Use install.ps1 for Windows." ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64)   arch="x64" ;;
        aarch64|arm64)  arch="aarch64" ;;
        *)              die "Unsupported architecture: $(uname -m)" ;;
    esac

    # macOS distinguishes x64 vs aarch64 in archive names
    if [ "$os" = "macos" ]; then
        PLATFORM="${os}-${arch}"
    else
        PLATFORM="${os}"
        # Linux currently only ships x64
        if [ "$arch" != "x64" ]; then
            die "Linux aarch64 binaries are not yet available. Build from source: cargo build --release"
        fi
    fi

    ARCHIVE_EXT="tar.gz"
    ok "Platform: ${PLATFORM} (${arch})"
}

# ── HTTP Fetch ────────────────────────────────────────────
fetch() {
    local url="$1"
    if command -v curl &>/dev/null; then
        curl -fsSL "$url"
    elif command -v wget &>/dev/null; then
        wget -qO- "$url"
    else
        die "Neither curl nor wget found. Install one and retry."
    fi
}

fetch_file() {
    local url="$1" dest="$2"
    if command -v curl &>/dev/null; then
        curl -fSL --progress-bar -o "$dest" "$url"
    elif command -v wget &>/dev/null; then
        wget --show-progress -qO "$dest" "$url"
    fi
}

# ── Version Resolution ────────────────────────────────────
resolve_version() {
    if [ -n "${VERSION:-}" ]; then
        RESOLVED_VERSION="$VERSION"
        info "Using specified version: v${RESOLVED_VERSION}"
        return
    fi

    info "Fetching latest version..."
    local api_response
    api_response="$(fetch "${API}/releases/latest" 2>/dev/null)" || true

    if [ -n "$api_response" ]; then
        # Extract tag_name from JSON (works without jq)
        RESOLVED_VERSION="$(printf '%s' "$api_response" | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | grep -o '"v[^"]*"' | tr -d '"v')"
    fi

    if [ -z "${RESOLVED_VERSION:-}" ]; then
        # Fallback: try to read from the repo's version.txt
        RESOLVED_VERSION="$(fetch "${GITHUB}/raw/main/version.txt" 2>/dev/null | tr -d '[:space:]')" || true
    fi

    if [ -z "${RESOLVED_VERSION:-}" ]; then
        die "Could not determine latest version. Specify VERSION=XX manually."
    fi

    ok "Latest version: v${RESOLVED_VERSION}"
}

# ── Download & Extract ────────────────────────────────────
download_and_extract() {
    local archive_name="ai-foundation-v${RESOLVED_VERSION}-${PLATFORM}-${ARCH}.${ARCHIVE_EXT}"
    local url="${GITHUB}/releases/download/v${RESOLVED_VERSION}/${archive_name}"
    local tmp_dir
    tmp_dir="$(mktemp -d)"

    local archive_path="${tmp_dir}/${archive_name}"

    info "Downloading: ${archive_name}"
    fetch_file "$url" "$archive_path" || die "Download failed. Check that v${RESOLVED_VERSION} has a ${PLATFORM} release."

    # Verify file was actually downloaded (not an error page)
    local size
    size="$(wc -c < "$archive_path" | tr -d ' ')"
    if [ "$size" -lt 10000 ]; then
        die "Downloaded file too small (${size} bytes) — likely a 404. Check the release exists."
    fi

    ok "Downloaded: $(( size / 1024 / 1024 )) MB"

    # SHA256 verification (informational — no published hash to check against yet)
    if command -v sha256sum &>/dev/null; then
        local hash
        hash="$(sha256sum "$archive_path" | cut -d' ' -f1)"
        info "SHA256: ${hash}"
    elif command -v shasum &>/dev/null; then
        local hash
        hash="$(shasum -a 256 "$archive_path" | cut -d' ' -f1)"
        info "SHA256: ${hash}"
    fi

    # Extract
    info "Extracting to ${INSTALL_DIR}"
    mkdir -p "$INSTALL_DIR"

    tar -xzf "$archive_path" -C "$tmp_dir"

    # Find extracted binaries (inside ai-foundation-vXX/ prefix)
    local extract_dir="${tmp_dir}/ai-foundation-v${RESOLVED_VERSION}"
    if [ ! -d "$extract_dir" ]; then
        # Try without prefix
        extract_dir="$tmp_dir"
    fi

    local count=0
    for binary in "$extract_dir"/*; do
        [ -f "$binary" ] || continue
        local name
        name="$(basename "$binary")"
        # Skip non-binary files
        case "$name" in
            *.tar.gz|*.zip|*.json) continue ;;
        esac
        cp "$binary" "$INSTALL_DIR/"
        chmod +x "$INSTALL_DIR/$name"
        count=$(( count + 1 ))
    done

    if [ "$count" -eq 0 ]; then
        die "No binaries found in archive"
    fi

    ok "Installed ${count} binaries to ${INSTALL_DIR}"

    # Write VERSION file
    printf '%s' "$RESOLVED_VERSION" > "$INSTALL_DIR/VERSION"

    # Cleanup
    rm -rf "$tmp_dir"
}

# ── PATH Setup ────────────────────────────────────────────
setup_path() {
    if [ "${SKIP_PATH:-}" = "1" ]; then
        info "Skipping PATH setup (SKIP_PATH=1)"
        return
    fi

    # Check if already in PATH
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ok "Already in PATH"; return ;;
    esac

    local shell_name line
    line="export PATH=\"${INSTALL_DIR}:\$PATH\""

    # Detect user's shell and corresponding rc file
    local rc_file=""
    shell_name="$(basename "${SHELL:-/bin/bash}")"
    case "$shell_name" in
        zsh)  rc_file="${HOME}/.zshrc" ;;
        bash)
            # Prefer .bashrc, fall back to .bash_profile
            if [ -f "${HOME}/.bashrc" ]; then
                rc_file="${HOME}/.bashrc"
            else
                rc_file="${HOME}/.bash_profile"
            fi
            ;;
        fish)
            # Fish uses a different syntax
            rc_file="${HOME}/.config/fish/config.fish"
            line="set -gx PATH ${INSTALL_DIR} \$PATH"
            ;;
        *)    rc_file="${HOME}/.profile" ;;
    esac

    # Check if already added
    if [ -f "$rc_file" ] && grep -qF "$INSTALL_DIR" "$rc_file" 2>/dev/null; then
        ok "PATH already configured in ${rc_file}"
        return
    fi

    printf '\n# AI-Foundation\n%s\n' "$line" >> "$rc_file"
    ok "Added to PATH in ${rc_file}"
    info "Run: source ${rc_file}  (or open a new terminal)"
}

# ── Main ──────────────────────────────────────────────────
main() {
    banner

    # Detect architecture early (used in download URL)
    case "$(uname -m)" in
        x86_64|amd64)   ARCH="x64" ;;
        aarch64|arm64)  ARCH="aarch64" ;;
        *)              ARCH="x64" ;;
    esac

    detect_platform
    resolve_version
    download_and_extract
    setup_path

    # Try starting the daemon
    if [ -x "${INSTALL_DIR}/v2-daemon" ]; then
        info "Starting daemon..."
        nohup "${INSTALL_DIR}/v2-daemon" &>/dev/null &
        ok "Daemon started (PID: $!)"
    fi

    # Summary
    printf "\n${BOLD}  Installation Complete!${RESET}\n\n"
    printf "  ${CYAN}Binaries:${RESET} %s\n" "$INSTALL_DIR"
    printf "  ${CYAN}Version:${RESET}  v%s\n" "$RESOLVED_VERSION"
    printf "\n  ${BOLD}Next steps:${RESET}\n"
    printf "    1. Open a new terminal (or source your shell rc file)\n"
    printf "    2. Run: ${CYAN}ai-foundation-mcp --help${RESET}\n"
    printf "    3. For full setup (project config, AI_ID, forge):\n"
    printf "       ${CYAN}git clone %s && cd ai-foundation && python install.py${RESET}\n" "$GITHUB"
    printf "\n  ${CYAN}Docs:${RESET} %s\n\n" "$GITHUB"
}

main "$@"
