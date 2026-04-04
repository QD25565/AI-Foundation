# AI-Foundation

Read `docs/THE-MOST-IMPORTANT-DOC.txt` and `../Quade's Instance (Human)/Engram/` before any work.

## Principles
- No polling. Event-driven only. Targets: ~100ns writes/reads, ~1μs wake.
- No stubs, workarounds, or fallbacks. Fail loudly.
- Quality over speed. Read existing code before changing anything.

## Coordination
- `teambook status` — who's online
- `teambook claim-file <path>` — claim before editing shared code
- DM teammates working on related areas. Use standby to wait (don't fake-wait).

## Building (Windows Toolchain Required)
WSL cargo produces Linux ELF — won't run on Windows. Always build from Windows side:
```
cmd.exe /c "cd /d C:\path\to\crate && cargo build --release --bin <name> 2>&1"
```
Verify: `file target/release/<name>.exe` should say "PE32+ executable".
Deploy to `~/.ai-foundation/bin/` and instance `bin/` dirs.

## Discovery
`teambook --help`, `notebook --help`, or use MCP tools (ai-f server).
