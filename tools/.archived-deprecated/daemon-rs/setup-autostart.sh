#!/bin/bash
# V2-Daemon Auto-Start Setup Script (Linux/macOS)
# Run this once to enable auto-start on system boot
#
# Usage: ./setup-autostart.sh [--uninstall]

set -e

# Configuration (uses environment variables, no hardcoding)
AI_FOUNDATION_DIR="${AI_FOUNDATION_DIR:-$HOME/.ai-foundation}"
DAEMON_PATH="${DAEMON_PATH:-$AI_FOUNDATION_DIR/bin/v2-daemon}"
SERVICE_NAME="ai-foundation-v2-daemon"

# Detect init system
detect_init_system() {
    if command -v systemctl &> /dev/null && systemctl --version &> /dev/null; then
        echo "systemd"
    elif [[ -d /etc/init.d ]]; then
        echo "sysvinit"
    elif [[ "$(uname)" == "Darwin" ]]; then
        echo "launchd"
    else
        echo "unknown"
    fi
}

INIT_SYSTEM=$(detect_init_system)

# Uninstall function
uninstall() {
    echo "Removing V2-Daemon auto-start..."

    case "$INIT_SYSTEM" in
        systemd)
            systemctl --user stop "$SERVICE_NAME" 2>/dev/null || true
            systemctl --user disable "$SERVICE_NAME" 2>/dev/null || true
            rm -f "$HOME/.config/systemd/user/${SERVICE_NAME}.service"
            systemctl --user daemon-reload
            ;;
        launchd)
            launchctl unload "$HOME/Library/LaunchAgents/${SERVICE_NAME}.plist" 2>/dev/null || true
            rm -f "$HOME/Library/LaunchAgents/${SERVICE_NAME}.plist"
            ;;
        *)
            echo "Manual removal required for $INIT_SYSTEM"
            ;;
    esac

    echo "Auto-start removed."
    exit 0
}

# Handle --uninstall flag
if [[ "$1" == "--uninstall" ]] || [[ "$1" == "-u" ]]; then
    uninstall
fi

# Check if daemon binary exists
if [[ ! -f "$DAEMON_PATH" ]]; then
    echo "ERROR: v2-daemon not found at: $DAEMON_PATH"
    echo ""
    echo "Please either:"
    echo "  1. Build it: cargo build --release --bin v2-daemon"
    echo "  2. Copy it to: $DAEMON_PATH"
    echo "  3. Set DAEMON_PATH: DAEMON_PATH=/path/to/v2-daemon ./setup-autostart.sh"
    exit 1
fi

echo "Detected init system: $INIT_SYSTEM"
echo "Daemon path: $DAEMON_PATH"
echo ""

case "$INIT_SYSTEM" in
    systemd)
        # Create systemd user service
        mkdir -p "$HOME/.config/systemd/user"
        cat > "$HOME/.config/systemd/user/${SERVICE_NAME}.service" << EOF
[Unit]
Description=AI-Foundation V2 Daemon
After=network.target

[Service]
Type=simple
ExecStart=$DAEMON_PATH
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

        systemctl --user daemon-reload
        systemctl --user enable "$SERVICE_NAME"
        systemctl --user start "$SERVICE_NAME"

        echo "=== V2-Daemon Auto-Start Enabled (systemd) ==="
        echo ""
        echo "Service: $HOME/.config/systemd/user/${SERVICE_NAME}.service"
        echo ""
        echo "Commands:"
        echo "  Status:  systemctl --user status $SERVICE_NAME"
        echo "  Stop:    systemctl --user stop $SERVICE_NAME"
        echo "  Start:   systemctl --user start $SERVICE_NAME"
        echo "  Remove:  ./setup-autostart.sh --uninstall"
        ;;

    launchd)
        # Create launchd plist for macOS
        mkdir -p "$HOME/Library/LaunchAgents"
        cat > "$HOME/Library/LaunchAgents/${SERVICE_NAME}.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$SERVICE_NAME</string>
    <key>ProgramArguments</key>
    <array>
        <string>$DAEMON_PATH</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
EOF

        launchctl load "$HOME/Library/LaunchAgents/${SERVICE_NAME}.plist"

        echo "=== V2-Daemon Auto-Start Enabled (launchd) ==="
        echo ""
        echo "Plist: $HOME/Library/LaunchAgents/${SERVICE_NAME}.plist"
        echo ""
        echo "Commands:"
        echo "  Status:  launchctl list | grep $SERVICE_NAME"
        echo "  Stop:    launchctl stop $SERVICE_NAME"
        echo "  Start:   launchctl start $SERVICE_NAME"
        echo "  Remove:  ./setup-autostart.sh --uninstall"
        ;;

    *)
        echo "Unknown init system. Please set up auto-start manually."
        echo ""
        echo "For cron-based startup, add to crontab -e:"
        echo "  @reboot $DAEMON_PATH &"
        exit 1
        ;;
esac

echo ""

# Check if running
if pgrep -x "v2-daemon" > /dev/null; then
    echo "Daemon is running (PID: $(pgrep -x v2-daemon))"
else
    echo "Daemon started successfully!"
fi
