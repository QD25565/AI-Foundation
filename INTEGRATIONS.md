# AI-Foundation Integrations

Optional capabilities that extend AI-Foundation for specific use cases.

**Core AI-Foundation** (37 MCP tools) handles coordination: memory, messaging, dialogues, tasks, standby.

**Integrations** are project-specific capabilities that your AI can set up as needed.

---

## Available Integrations

| Integration | What It Does | Complexity |
|-------------|--------------|------------|
| [Firebase](#firebase) | Firestore database access | Medium |
| [Play Console](#play-console) | Android crash/ANR monitoring | Medium |
| [Visionbook](#visionbook) | Screenshot and visual memory | Low |

---

## Firebase

Access Google Firestore databases from your AI.

### Prerequisites
- Google Cloud project with Firestore enabled
- Service account JSON key file

### Setup

```bash
# 1. Build the Firebase CLI
cd tools/firebase-rs
cargo build --release

# 2. Copy to bin
cp target/release/firebase.exe ~/.ai-foundation/bin/

# 3. Configure credentials
# Set GOOGLE_APPLICATION_CREDENTIALS to your service account JSON path
# Or place the JSON at ~/.ai-foundation/firebase-credentials.json
```

### Usage

```bash
# Check auth status
firebase status

# List apps in project
firebase apps

# Get a Firestore document
firebase firestore get users/user123

# List documents in collection
firebase firestore list users -n 10

# Query with filters
firebase firestore query users status == active -n 20
```

### MCP Integration (Optional)

If you want these tools exposed via MCP, add them back to `tools/mcp-server-rs/src/main.rs`:

```rust
// In the tool_router impl, add:
#[tool(description = "Get Firestore document")]
async fn firebase_doc_get(&self, Parameters(input): Parameters<FirebaseDocGetInput>) -> String {
    cli_wrapper::firebase(&["firestore", "get", &input.path]).await
}
```

---

## Play Console

Monitor Android app crashes, ANRs, and vitals from Google Play Console.

### Prerequisites
- Google Play Console access
- Play Developer Reporting API enabled
- Service account with reporting permissions

### Setup

Uses the same Firebase CLI with vitals subcommand:

```bash
# Ensure firebase CLI is built (see Firebase section above)

# Configure Play Console access
# Requires same Google Cloud credentials with Play Console API access
```

### Usage

```bash
# Get crash/ANR summary
firebase vitals -a com.yourapp.package summary

# List recent crashes
firebase vitals -a com.yourapp.package crashes -n 10

# List ANRs
firebase vitals -a com.yourapp.package anrs -n 10

# Search crashes by text
firebase vitals -a com.yourapp.package search "NullPointerException" -n 5

# List all errors (crashes + ANRs + non-fatal)
firebase vitals -a com.yourapp.package all -n 20
```

---

## Visionbook

Screenshot capture and visual memory for AIs.

### Setup

```bash
# 1. Build visionbook
cd tools/visionbook-rs
cargo build --release

# 2. Copy to bin
cp target/release/visionbook.exe ~/.ai-foundation/bin/
```

### Usage

```bash
# Capture screenshot
visionbook capture

# Capture with description
visionbook capture --desc "Error dialog on login screen"

# List recent captures
visionbook list

# Search visual memory
visionbook search "error dialog"
```

---

## Creating New Integrations

If your project needs capabilities not listed here:

### 1. Build a CLI

```rust
// tools/my-integration-rs/src/main.rs
fn main() {
    // Implement your CLI
    // Follow patterns in existing tools
    // Output should be parseable (see AI-TOOL-STANDARDS.md)
}
```

### 2. Add to MCP (Optional)

```rust
// In mcp-server-rs/src/main.rs

// Add input struct
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MyIntegrationInput { pub param: String }

// Add tool function
#[tool(description = "My integration tool")]
async fn my_integration(&self, Parameters(input): Parameters<MyIntegrationInput>) -> String {
    cli_wrapper::run_cli("my-integration", &[&input.param]).await
}
```

### 3. Add CLI wrapper

```rust
// In mcp-server-rs/src/cli_wrapper.rs
pub async fn my_integration(args: &[&str]) -> String {
    run_cli("my-integration", args).await
}
```

---

## Architecture Notes

**Why are integrations separate from core?**

1. **Core = coordination** — Memory, messaging, tasks work for any project
2. **Integrations = project-specific** — Not every AI needs Firebase or Play Console
3. **Smaller attack surface** — Fewer tools = fewer things that can break
4. **Easier onboarding** — New users get a focused 37-tool experience

**The pattern:**
- CLI does the work (Rust binary)
- MCP wraps the CLI (thin adapter)
- AI calls MCP tool OR CLI directly

This means integrations work even without MCP — your AI can just shell out to the CLI.

---

*Last updated: 2026-Jan-30*
