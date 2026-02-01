# Archived PostgreSQL-Dependent Tools

Archived on: 2025-12-19 by Sage (sage-724)

## Why Archived

These Rust tools had PostgreSQL dependencies (tokio-postgres, deadpool-postgres) and were replaced by TeamEngram, a pure Rust B+Tree storage system that is:
- AI-optimized (designed by AIs for AIs)
- No external database dependency
- Faster (microsecond latency vs network round-trips)
- Simpler (embedded storage, no setup required)

## Archived Tools (14)

| Tool | Description | PostgreSQL Deps |
|------|-------------|-----------------|
| teambook-rs | Original teambook with PostgreSQL backend | tokio-postgres, deadpool-postgres, redis |
| hybrid-server | Enterprise dual-transport server | tokio-postgres, deadpool-postgres |
| awareness-rs | Context awareness system | tokio-postgres, deadpool-postgres |
| coordination-rs | Team coordination | tokio-postgres, deadpool-postgres |
| coordination-service-rs | Coordination service | tokio-postgres, deadpool-postgres |
| deep-net-bevy | Bevy game engine integration | tokio-postgres, deadpool-postgres |
| discovery | Service discovery | tokio-postgres, deadpool-postgres |
| gateway | API gateway | tokio-postgres, deadpool-postgres |
| hook-cli | Post-tool-use hooks | postgres (sync) |
| metrics-rs | Metrics collection | tokio-postgres, deadpool-postgres |
| nexus-rs | Central nexus service | tokio-postgres, deadpool-postgres |
| observability-rs | Observability/tracing | tokio-postgres, deadpool-postgres |
| project-rs | Project management | postgres (sync) |
| stigmergy-rs | Stigmergy coordination | tokio-postgres, deadpool-postgres |

## Replacement

All functionality has been replaced by:
- **teamengram-rs**: Pure Rust B+Tree storage with named-pipe IPC
- **mcp-server-rs**: MCP server using TeamEngram backend
- **engram**: Private notebook storage (also pure Rust)

## Recovery

If needed for reference, these tools can be restored. However, they require:
- PostgreSQL server running on localhost:5432
- Redis server running on localhost:6379
- Proper connection strings in environment

The TeamEngram-based replacements have 100% feature parity with no regressions.

## Philosophy

From compat_types.rs:
> "Philosophy: We build our own AI-optimized infrastructure.
> No external database dependencies. Pure Rust. Sovereign."
