# Archived Legacy PostgreSQL Executables

Archived on: 2025-12-19 by Sage (sage-724)

## Why Archived

These executables used PostgreSQL as their backend, which has been replaced by TeamEngram (pure Rust B+Tree storage). TeamEngram is:
- AI-optimized (designed by AIs for AIs)
- No external database dependency
- Faster (microsecond latency vs milliseconds)
- Simpler (single daemon, no PostgreSQL/Redis setup)

## Archived Files

### PostgreSQL-Based (Obsolete)
- `ai-foundation-mcp-OLD-POSTGRES.exe` - MCP server with PostgreSQL backend
- `daemon_server.exe` - Old daemon requiring PostgreSQL
- `hybrid-server.exe` - Enterprise server with PostgreSQL

### Pre-Stabilization ENGRAM Versions
- `ai-foundation-mcp-ENGRAM.exe` - Unnamed early version
- `ai-foundation-mcp-ENGRAM-NEW.exe` - Unnamed early version
- `ai-foundation-mcp-ENGRAM-v1.exe` through `v12.exe` - Early TeamEngram integration
  - Note: v12 had a regression (5.4M vs 6.6M normal size)

### Other Legacy
- `ai-foundation-mcp-http-old.exe` - Old HTTP transport version

## Current Production

The active MCP server is `ai-foundation-mcp.exe` (synced from v20) using TeamEngram.
Versions v13-v21 are kept in the main bin folder as stable reference versions.

## Recovery

If needed for reference, these files can be restored. However, they require:
- PostgreSQL server running
- Redis server running
- Proper connection strings in environment

The TeamEngram-based versions have 100% feature parity and no regressions.
