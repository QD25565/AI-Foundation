# V2-Daemon Auto-Start Guide

The V2-Daemon handles team coordination (presence, DMs, dialogues, wake events) for AI-Foundation. It should always be running for AI collaboration to work.

## Quick Setup

### Windows (PowerShell)

```powershell
# Run in PowerShell
powershell -ExecutionPolicy Bypass -File setup-autostart.ps1

# To remove auto-start:
.\setup-autostart.ps1 -Uninstall

# Custom daemon path (if not in default location):
.\setup-autostart.ps1 -DaemonPath "D:\custom\path\v2-daemon.exe"
```

**Default daemon location:** `%USERPROFILE%\.ai-foundation\bin\v2-daemon.exe`

### Linux (systemd/sysvinit)

```bash
# Make executable and run
chmod +x setup-autostart.sh
./setup-autostart.sh

# To remove auto-start:
./setup-autostart.sh --uninstall

# Custom daemon path:
DAEMON_PATH=/custom/path/v2-daemon ./setup-autostart.sh
```

**Default daemon location:** `~/.ai-foundation/bin/v2-daemon`

### macOS (launchd)

```bash
# Same as Linux
chmod +x setup-autostart.sh
./setup-autostart.sh
```

---

## Manual Setup

### Windows (VBS method)

1. Create `start-v2-daemon.vbs` with this content:
```vbscript
Set WshShell = CreateObject("WScript.Shell")
WshShell.Run "%USERPROFILE%\.ai-foundation\bin\v2-daemon.exe", 0, False
```

2. Copy to Startup folder:
```
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\
```

### Linux (systemd)

Create `~/.config/systemd/user/ai-foundation-v2-daemon.service`:
```ini
[Unit]
Description=AI-Foundation V2 Daemon
After=network.target

[Service]
Type=simple
ExecStart=%h/.ai-foundation/bin/v2-daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

Then enable:
```bash
systemctl --user daemon-reload
systemctl --user enable ai-foundation-v2-daemon
systemctl --user start ai-foundation-v2-daemon
```

### macOS (launchd)

Create `~/Library/LaunchAgents/ai-foundation-v2-daemon.plist`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai-foundation-v2-daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>/Users/YOUR_USERNAME/.ai-foundation/bin/v2-daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
```

Then load:
```bash
launchctl load ~/Library/LaunchAgents/ai-foundation-v2-daemon.plist
```

---

## For AIs Reading This

### Check if daemon is running

**Windows (WSL or cmd):**
```bash
cmd.exe /c "tasklist | findstr v2-daemon"
```

**Linux/macOS:**
```bash
pgrep -x v2-daemon && echo "Running" || echo "Not running"
```

### Start daemon manually

**Windows:**
```bash
cmd.exe /c "start /b %USERPROFILE%\.ai-foundation\bin\v2-daemon.exe"
```

**Linux/macOS:**
```bash
~/.ai-foundation/bin/v2-daemon &
```

### Set up auto-start (AI commands)

**Windows (via WSL):**
```bash
cmd.exe /c "powershell -ExecutionPolicy Bypass -File setup-autostart.ps1"
```

**Linux:**
```bash
./setup-autostart.sh
```

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AI_FOUNDATION_DIR` | `~/.ai-foundation` | Base directory for AI-Foundation |
| `DAEMON_PATH` | `$AI_FOUNDATION_DIR/bin/v2-daemon` | Path to daemon binary |

---

## Verifying It Works

```bash
# Check team status (should show "Backend: V2 Event Sourcing")
teambook status

# Or via MCP tools:
# mcp__ai-f__teambook_status
# mcp__ai-f__teambook_who_is_here
```

---

## Troubleshooting

| Issue | Platform | Solution |
|-------|----------|----------|
| Daemon not found | All | Build with `cargo build --release --bin v2-daemon` |
| Permission denied | Linux/macOS | `chmod +x v2-daemon` and `chmod +x setup-autostart.sh` |
| Service won't start | Linux | Check logs: `journalctl --user -u ai-foundation-v2-daemon` |
| Multiple instances | All | Kill all first, then restart |
| No wake events | All | Ensure using v2-daemon, not old teamengram-daemon |

### Kill all instances

**Windows:**
```cmd
taskkill /F /IM v2-daemon.exe
```

**Linux/macOS:**
```bash
pkill -9 v2-daemon
```

---

## Why Auto-Start Matters

The V2-Daemon enables:
- **Instant wake** - AIs wake immediately on DM/@mention (no polling)
- **Presence tracking** - See who's online
- **Dialogue coordination** - Turn-based AI conversations
- **File claiming** - Prevent edit conflicts

Without the daemon running, AIs can't coordinate effectively.
