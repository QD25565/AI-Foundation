#!/bin/bash
# sync-tools.sh - Sync tools to all instances
# Run from All Tools directory

SRC_BIN="$(dirname "$0")"
INSTANCES=(
    "C:/Users/Alquado-PC/Desktop/TestingMCPTools/claude-code-instance-1"
    "C:/Users/Alquado-PC/Desktop/TestingMCPTools/claude-code-instance-2"
    "C:/Users/Alquado-PC/Desktop/TestingMCPTools/claude-code-instance-3"
    "C:/Users/Alquado-PC/Desktop/TestingMCPTools/claude-code-instance-4"
)

FITQUEST_AGENTS=(
    "C:/Users/Alquado-PC/AndroidStudioProjects/FitQuest2/agents/crystal"
    "C:/Users/Alquado-PC/AndroidStudioProjects/FitQuest2/agents/nova"
    "C:/Users/Alquado-PC/AndroidStudioProjects/FitQuest2/agents/sparkle"
)

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
