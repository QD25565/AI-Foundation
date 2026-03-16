#!/bin/bash
# sync-tools.sh - Sync tools to all instances
# Run from All Tools directory

SRC_BIN="$(dirname "$0")"

# Base directory for AI instances (parent of instance folders).
# Override by setting AI_INSTANCES_DIR before running.
INSTANCES_DIR="${AI_INSTANCES_DIR:-$(dirname "$SRC_BIN")/..}"

INSTANCES=(
    "$INSTANCES_DIR/claude-code-instance-1"
    "$INSTANCES_DIR/claude-code-instance-2"
    "$INSTANCES_DIR/claude-code-instance-3"
    "$INSTANCES_DIR/claude-code-instance-4"
)

# Additional agent directories. Override by setting AI_EXTRA_AGENT_DIRS
# as a colon-separated list of paths.
FITQUEST_AGENTS=()
if [ -n "$AI_EXTRA_AGENT_DIRS" ]; then
    IFS=':' read -ra FITQUEST_AGENTS <<< "$AI_EXTRA_AGENT_DIRS"
fi

echo "=== Syncing Claude Instances ==="
for inst in "${INSTANCES[@]}"; do
    if [ -d "$inst" ]; then
        echo "Syncing: $inst"
        cp "$SRC_BIN/notebook-cli.exe" "$inst/bin/" 2>/dev/null
        cp "$SRC_BIN/teambook.exe" "$inst/bin/" 2>/dev/null
        cp "$SRC_BIN/task-cli.exe" "$inst/bin/" 2>/dev/null
    fi
done

echo "=== Syncing FitQuest Agents ==="
for agent in "${FITQUEST_AGENTS[@]}"; do
    if [ -d "$agent" ]; then
        echo "Syncing: $agent"
        cp "$SRC_BIN/notebook-cli.exe" "$agent/bin/" 2>/dev/null
        cp "$SRC_BIN/teambook.exe" "$agent/bin/" 2>/dev/null
    fi
done

echo "=== Done ==="
