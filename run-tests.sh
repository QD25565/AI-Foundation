#!/usr/bin/env bash
#
# AI-Foundation build → test → deploy pipeline.
#
# Pipeline:
#   1. Build  — cargo build --release (daemon + teambook)
#   2. Test   — run integration tests against the BUILT binary (not yet deployed)
#   3. Deploy — if and only if tests pass, copy to ~/.ai-foundation/bin/
#
# AIs never receive an untested binary. If tests fail, deploy is blocked.
#
# Usage:
#   ./run-tests.sh              # full pipeline: build → test → deploy
#   ./run-tests.sh --no-build   # skip build, test existing release output, deploy if green
#   ./run-tests.sh --no-deploy  # build + test only, no deploy (dry run)
#   ./run-tests.sh --threads N  # parallel test threads (default: 4)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEAMENGRAM_DIR="$SCRIPT_DIR/tools/teamengram-rs"
TEST_DIR="$SCRIPT_DIR/tools/ai-foundation-tests"
RELEASE_DIR="$TEAMENGRAM_DIR/target/release"
DEPLOY_DIR="$HOME/.ai-foundation/bin"

NO_BUILD=false
NO_DEPLOY=false
UPDATE_GOLDEN=false
THREADS=4

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-build)   NO_BUILD=true;  shift ;;
        --no-deploy)      NO_DEPLOY=true;      shift ;;
        --update-golden)  UPDATE_GOLDEN=true;  shift ;;
        --threads)        THREADS="$2";       shift 2 ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# Convert a WSL path to a Windows path for cmd.exe
to_win() { wslpath -w "$1" 2>/dev/null || echo "$1"; }

WIN_TEAMENGRAM=$(to_win "$TEAMENGRAM_DIR")
WIN_TEST=$(to_win "$TEST_DIR")

echo "=========================================="
echo "  AI-Foundation Build → Test → Deploy"
echo "  $(date -u '+%Y-%m-%d %H:%M UTC')"
echo "=========================================="
echo ""

# ── 1. Build ──────────────────────────────────────────────────────────────────
if [[ "$NO_BUILD" == false ]]; then
    echo ">>> [1/3] Building teamengram-daemon (release)..."
    cmd.exe /c "cd /d \"$WIN_TEAMENGRAM\" && cargo build --release --bin teamengram-daemon 2>&1" || {
        echo "FAIL: daemon build failed — pipeline stopped"
        exit 1
    }
    echo ""

    echo ">>> [1/3] Building teambook-engram (release)..."
    cmd.exe /c "cd /d \"$WIN_TEAMENGRAM\" && cargo build --release --bin teambook-engram 2>&1" || {
        echo "FAIL: teambook build failed — pipeline stopped"
        exit 1
    }
    echo ""

    echo ">>> [1/3] Building v2-daemon (release)..."
    cmd.exe /c "cd /d \"$WIN_TEAMENGRAM\" && cargo build --release --bin v2-daemon 2>&1" || {
        echo "FAIL: v2-daemon build failed — pipeline stopped"
        exit 1
    }
    echo ""
else
    echo ">>> [1/3] Build skipped (--no-build)"
    echo ""
fi

# ── Resolve built binaries ─────────────────────────────────────────────────────
DAEMON_EXE="$RELEASE_DIR/teamengram-daemon.exe"
TEAMBOOK_EXE="$RELEASE_DIR/teambook-engram.exe"
V2_DAEMON_EXE="$RELEASE_DIR/v2-daemon.exe"

[[ -f "$DAEMON_EXE" ]]     || { echo "ERROR: $DAEMON_EXE not found. Run without --no-build."; exit 1; }
[[ -f "$TEAMBOOK_EXE" ]]   || { echo "ERROR: $TEAMBOOK_EXE not found. Run without --no-build."; exit 1; }
[[ -f "$V2_DAEMON_EXE" ]]  || { echo "ERROR: $V2_DAEMON_EXE not found. Run without --no-build."; exit 1; }

DAEMON_WIN=$(to_win "$DAEMON_EXE")
TEAMBOOK_WIN=$(to_win "$TEAMBOOK_EXE")
V2_DAEMON_WIN=$(to_win "$V2_DAEMON_EXE")

echo "Daemon:    $DAEMON_WIN"
echo "Teambook:  $TEAMBOOK_WIN"
echo "V2 Daemon: $V2_DAEMON_WIN"
echo "Threads:  $THREADS"
echo ""

# ── 2. Test ───────────────────────────────────────────────────────────────────
echo ">>> [2/3] Running integration tests against built binary..."
echo "    (AIs never see these binaries until this step passes)"
echo ""

# Set UPDATE_GOLDEN_VAL for cmd.exe (1 = write golden files, empty = compare)
UPDATE_GOLDEN_VAL=""
[[ "${UPDATE_GOLDEN}" == "true" ]] && UPDATE_GOLDEN_VAL="1"
echo ""

cmd.exe /c "cd /d \"$WIN_TEST\" && \
    set AIF_DAEMON_BIN=$DAEMON_WIN&& \
    set AIF_TEAMBOOK_BIN=$TEAMBOOK_WIN&& \
    set AIF_V2_DAEMON_BIN=$V2_DAEMON_WIN&& \
    set UPDATE_GOLDEN=$UPDATE_GOLDEN_VAL&& \
    cargo test -- --test-threads=$THREADS 2>&1"
TEST_EXIT=$?

echo ""
if [[ $TEST_EXIT -ne 0 ]]; then
    echo "=========================================="
    echo "  FAILED — deploy blocked"
    echo "  Fix the regression, then re-run."
    echo "=========================================="
    exit $TEST_EXIT
fi

echo "=========================================="
echo "  Tests PASSED"
echo "=========================================="
echo ""

# ── 3. Deploy ─────────────────────────────────────────────────────────────────
if [[ "$NO_DEPLOY" == true ]]; then
    echo ">>> [3/3] Deploy skipped (--no-deploy)"
    echo "    Binaries are ready at: $RELEASE_DIR"
else
    echo ">>> [3/3] Deploying to $DEPLOY_DIR ..."
    mkdir -p "$DEPLOY_DIR"

    cp "$DAEMON_EXE"     "$DEPLOY_DIR/teamengram-daemon.exe"
    cp "$TEAMBOOK_EXE"   "$DEPLOY_DIR/teambook.exe"
    cp "$V2_DAEMON_EXE"  "$DEPLOY_DIR/v2-daemon.exe"

    echo "    teamengram-daemon.exe -> $DEPLOY_DIR/"
    echo "    teambook-engram.exe   -> $DEPLOY_DIR/teambook.exe"
    echo "    v2-daemon.exe         -> $DEPLOY_DIR/"
    echo ""
    echo "    AIs will pick up the updated binaries on their next session."
fi

echo ""
echo "=========================================="
echo "  Pipeline complete"
echo "=========================================="
exit 0
