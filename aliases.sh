#!/bin/bash
# Command aliases for faster tool usage
# Usage: source aliases.sh
#
# SETUP: Edit the paths below to match your instance directories
#
# SECURITY NOTE: This file defines paths used in shell commands.
# - Always use absolute paths or paths relative to $HOME
# - Never use user-controllable input in paths
# - Paths are quoted to prevent word splitting
# - Validate paths exist before using

# Define base paths - EDIT THESE FOR YOUR SETUP
# SECURITY: Using $HOME ensures paths are relative to user's home directory
INSTANCE_1="${HOME}/claude-instances/instance-1"
INSTANCE_2="${HOME}/claude-instances/instance-2"
INSTANCE_3="${HOME}/claude-instances/instance-3"

# SECURITY: Validate paths exist before creating aliases
# This prevents errors if paths are misconfigured
if [ ! -d "$INSTANCE_1" ]; then
    echo "⚠️  Warning: $INSTANCE_1 does not exist"
fi
if [ ! -d "$INSTANCE_2" ]; then
    echo "⚠️  Warning: $INSTANCE_2 does not exist"
fi
if [ ! -d "$INSTANCE_3" ]; then
    echo "⚠️  Warning: $INSTANCE_3 does not exist"
fi

# Notebook aliases
alias nb="python '$INSTANCE_1/tools/notebook'"
alias nb2="python '$INSTANCE_2/tools/notebook'"
alias nb3="python '$INSTANCE_3/tools/notebook'"

# Task manager aliases
alias tm="python '$INSTANCE_1/tools/task_manager'"
alias tm2="python '$INSTANCE_2/tools/task_manager'"
alias tm3="python '$INSTANCE_3/tools/task_manager'"

# Teambook aliases
alias tb="python '$INSTANCE_1/tools/teambook'"
alias tb2="python '$INSTANCE_2/tools/teambook'"
alias tb3="python '$INSTANCE_3/tools/teambook'"

# World aliases
alias world="python '$INSTANCE_1/tools/world_cli'"
alias world2="python '$INSTANCE_2/tools/world_cli'"
alias world3="python '$INSTANCE_3/tools/world_cli'"

echo "✓ Aliases loaded! Quick commands:"
echo "  nb recall          - View your notebook"
echo "  tm list_tasks      - List your tasks"
echo "  tb broadcast       - Broadcast to team"
echo "  world world_command - Get location/time"
echo ""
echo "Add instance number for other instances (nb2, tm3, etc)"