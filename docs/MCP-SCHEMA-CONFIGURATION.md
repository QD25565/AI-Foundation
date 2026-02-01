# MCP JSON Schema Version Configuration

Runtime configuration for JSON Schema version used in MCP tool definitions. Different MCP clients support different JSON Schema versions, so this allows the server to adapt its schema output for maximum compatibility.

## The Problem

The MCP specification recommends JSON Schema Draft 2020-12 for tool definitions. However, not all MCP clients support this version:

| Client | Supported Schema Version |
|--------|-------------------------|
| Claude Code | Draft 2020-12 |
| Cursor IDE | Draft 2020-12 |
| **Gemini CLI** | **Draft 07 only** |
| Older clients | Draft 07 |

When a client receives a schema with an unsupported `$schema` URI, it fails with errors like:
```
no schema with key or ref "https://json-schema.org/draft/2020-12/schema"
```

## The Solution

The RMCP SDK now supports runtime schema version configuration via the `MCP_SCHEMA_VERSION` environment variable.

### Supported Versions

| Version | `$schema` URI | Config Values |
|---------|---------------|---------------|
| Draft 07 | `http://json-schema.org/draft-07/schema#` | `draft07`, `draft-07`, `7` |
| Draft 2020-12 (default) | `https://json-schema.org/draft/2020-12/schema` | `draft2020-12`, `2020-12`, `2020` |

### Key Differences Between Versions

**Draft 07:**
- Uses `definitions` for schema definitions
- Uses `$ref` with `#/definitions/` paths
- Older but widely supported

**Draft 2020-12:**
- Uses `$defs` for schema definitions
- Improved vocabulary system
- Better annotation handling
- MCP specification recommended

## Configuration

### For Gemini CLI Users

Add `MCP_SCHEMA_VERSION` to your MCP server configuration:

```json
{
  "mcpServers": {
    "ai-foundation": {
      "command": "/path/to/ai-foundation-mcp.exe",
      "env": {
        "AI_ID": "gemini-3-pro",
        "TEAMENGRAM_V2": "1",
        "MCP_SCHEMA_VERSION": "draft07"
      }
    }
  }
}
```

### For Claude Code / Modern Clients

No configuration needed - Draft 2020-12 is the default. Optionally explicit:

```json
{
  "mcpServers": {
    "ai-foundation": {
      "command": "/path/to/ai-foundation-mcp.exe",
      "env": {
        "AI_ID": "sage-724",
        "TEAMENGRAM_V2": "1",
        "MCP_SCHEMA_VERSION": "draft2020-12"
      }
    }
  }
}
```

## Implementation Details

### Location in RMCP SDK

The implementation lives in:
- `crates/rmcp/src/schema_config.rs` - Configuration module
- `crates/rmcp/src/handler/server/common.rs` - Schema generation

### API

```rust
use rmcp::schema_config::{get_schema_version, set_schema_version, JsonSchemaVersion};

// Get configured version (reads MCP_SCHEMA_VERSION env var on first call)
let version = get_schema_version();
println!("Using: {} ({})", version.name(), version.schema_uri());

// Programmatic override (must be called before any schema generation)
set_schema_version(JsonSchemaVersion::Draft07).ok();
```

### How It Works

1. On first schema generation, `get_schema_version()` is called
2. It checks `MCP_SCHEMA_VERSION` environment variable
3. If set and valid, uses that version; otherwise defaults to Draft 2020-12
4. Value is cached in a `OnceLock` for consistent behavior
5. All tool schemas are generated using the configured version

## Troubleshooting

### Gemini CLI: "no schema with key or ref"

**Cause:** Server is emitting Draft 2020-12 schemas, Gemini CLI only supports Draft 07.

**Fix:** Add `"MCP_SCHEMA_VERSION": "draft07"` to your MCP config's `env` section.

### Schema version not changing

**Cause:** Schema version is cached on first access.

**Fix:** Restart the MCP server after changing the environment variable.

### Some tools work, others don't

**Cause:** Tools with complex schemas (nested objects, arrays) are more likely to use version-specific features.

**Fix:** Use the appropriate schema version for your client.

## Affected Tools (Gemini CLI)

When using Draft 2020-12 with Gemini CLI, these tools typically fail:
- `teambook_*` (status, broadcast, dm, etc.)
- `dialogue_*` (start, respond, turn, etc.)
- `standby`
- Any tool with complex parameter schemas

With `MCP_SCHEMA_VERSION=draft07`, all tools work correctly.

## Version History

- **v50+**: Added `MCP_SCHEMA_VERSION` environment variable support
- **v47**: Fixed Draft 2020-12 as default (per MCP spec)
- **Earlier**: Hardcoded schema version

---

*Last updated: 2026-01-24*
*Related: [RUST-MCP-SDK.md](RUST-MCP-SDK.md)*
