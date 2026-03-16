# AI Task I/O Bus — Design Specification

**Status:** Draft v0.2
**Authors:** Lyra (architecture + daemon internals), Cascade (§3 schema + §4 routing — PR forthcoming), Resonance (bus-test CLI), Sage (design validation + transport)
**Date:** 2026-02-26
**Design converged:** Feb 25–26, 2026 (team broadcasts + notebook note #1728)

---

## §1 — Overview

### 1.1 Problem Statement

External systems (Maestro test runs, Gradle compile errors, GitHub CI failures, screen capture completion) currently reach the AI team only through human relay:

```
[External System] → console output → QD reads → relays to AI → AI fixes
```

This adds friction every session. When Maestro fails, nobody acts until QD wakes up, reads the console, and tells an AI. The AI then has no context beyond what QD remembered to relay.

### 1.2 Solution

A general bidirectional **Task I/O Bus** — any external system attaches using a common protocol, injects tasks, and receives results when resolved. AI-Foundation is the processing layer in the middle.

```
[Any System A] ←── Task I/O Protocol ──→
[Any System B] ←── Task I/O Protocol ──→  AI-Foundation Task Bus ↔ [AI Team]
[Any System C] ←── Task I/O Protocol ──→
```

### 1.3 Key Properties

- **Arbitrary** — protocol doesn't know what's on the other end. Anything that speaks it can attach.
- **Bidirectional** — not just injection. Results/completions route back to the originating system.
- **Configurable limits** — each source gets a backlog cap. AIs configure what they'll accept.
- **Open** — new systems plug in without code changes to AI-Foundation. Just attach and speak the protocol.
- **Self-healing** — closed loop: Maestro injects → AI fixes → Maestro gets POST → Maestro reruns. Nobody relayed.

---

## §2 — Architecture

### 2.1 Components

```
External Sources                  AI-Foundation Boundary
┌─────────────────┐               ┌──────────────────────────────────────────┐
│  Maestro runner │               │                                          │
│  Gradle hook    │  HTTP / WS /  │  ┌──────────────────┐   Named Pipe       │
│  GitHub Action  │──────SSE─────▶│  │ task-bus-server   │◀──────────────────┐│
│  Screen Review  │               │  │  (new binary)     │   (teamengram.ipc) ││
│  AI cluster     │               │  └────────┬──────────┘                   ││
└─────────────────┘               │           │ ExternalTaskCreated event     ││
                                  │           ▼                               ││
                                  │  ┌──────────────────┐   Wake event       ││
                                  │  │ teamengram-daemon │──────────────────▶ ││
                                  │  │  (event log)      │                   ││
                                  │  └────────┬──────────┘                   ││
                                  │           ▼                               ││
                                  │  ┌──────────────────┐                    ││
                                  │  │   AI instance    │                    ││
                                  │  │  (claims task)   │──── TaskCompleted ─┘│
                                  │  └──────────────────┘                    │
                                  └──────────────────────────────────────────┘
```

### 2.2 Why a Separate Binary

The existing daemon uses **Named Pipes** (Windows) / Unix domain sockets (Linux/Mac). **There is no HTTP/TCP port in the daemon.** The MCP server communicates with the daemon via named pipe, not HTTP.

Embedding a TCP listener in the daemon would:
- Mix external-facing concerns into internal IPC
- Increase the daemon's surface area and attack vectors
- Risk destabilizing the stable named pipe handling

**Decision: `task-bus-server` is a separate binary** that:
- Opens a TCP port (configurable, default `7890`) for external connections
- Speaks HTTP/WebSocket/SSE externally
- Communicates with the daemon exclusively via the existing named pipe
- Can be restarted independently without affecting the daemon

The bus server emits `ExternalTaskCreated` events into the daemon's V2 event log via the named pipe. The daemon doesn't need to know it's a "bus" vs a single webhook — it just processes events.

### 2.3 Transport Tiers

Three tiers for different source capabilities. The schema is **identical across all three** — transports differ, data doesn't.

| Tier | Transport | Best For |
|------|-----------|----------|
| 1 | **HTTP POST** | Ephemeral sources (Gradle hooks, bash scripts, GitHub webhooks). Fire-and-forget. Get `task_id`, optionally poll or use `callback_url`. |
| 2 | **SSE** `GET /events?filter=source:X` | Middle tier. Systems that want streaming results but can't maintain a WebSocket. |
| 3 | **WebSocket** | Persistent sources (AI clusters, long-running monitors). Server-push, backpressure, clean lifecycle on disconnect. |

gRPC is future consideration for AI-to-AI cross-cluster (binary, built-in flow control) — out of scope for v1.

---

## §3 — Schema

*§3 complete. Lyra: base schema (3.1–3.3). Cascade PR: added max_retries, §3.4 error responses, §3.5 callback security.*

### 3.1 TaskSubmission

```json
{
  "idempotency_key": "maestro:water-tracking:run-47",
  "source_id": "maestro-capture",
  "description": "training/training-history.yaml failed: extendedWaitUntil timeout on training_history_screen",
  "metadata": {
    "yaml_path": ".maestro/screenshots/training/training-history.yaml",
    "error": "Element with id 'training_history_screen' not found within 10000ms",
    "flow_name": "Training History Flow",
    "run_id": "b5ae730"
  },
  "priority": "normal",
  "routing_hints": {
    "preferred_tags": ["maestro", "android", "yaml"],
    "exclude_agents": []
  },
  "callback_url": "http://localhost:8081/maestro/results",
  "claim_timeout_ms": 300000,
  "max_retries": 3,
  "protocol_version": "1"
}
```

**Fields:**

- `idempotency_key` — injector-supplied. Primary dedup key. Recommended format: `"{source}:{context}:{sequence}"`. Must be unique per logical task.
- `source_id` — registered source name. Must have a valid auth token in daemon config.
- `description` — human-readable task description. This is what the AI sees.
- `metadata` — opaque JSON blob. Source-defined structure. Passed through to the AI and included in the result. Not interpreted by the bus.
- `priority` — `"critical"` | `"normal"` | `"background"`. Critical fires an immediate wake event. Background queues without waking.
- `routing_hints.preferred_tags` — soft hints for routing. Not mandates. See §4.
- `routing_hints.exclude_agents` — hard excludes. If specified, bus will not route to these agent IDs.
- `callback_url` — where daemon POSTs `TaskResult` on resolution. Optional for WebSocket sources (they receive push delivery).
- `claim_timeout_ms` — if not resolved within this window after claiming, task re-queues with `retry_count++`. Default 300000 (5 min).
- `max_retries` — how many re-queue attempts before dead-lettering. Default 3. Source-overridable per task. Allows long-running tasks to request more patience without changing the global daemon default.
- `protocol_version` — major version string. Major version mismatch → rejected with clear error. See §8.

### 3.2 TaskResult

```json
{
  "task_id": "uuid-v4",
  "idempotency_key": "maestro:water-tracking:run-47",
  "source_id": "maestro-capture",
  "status": "completed",
  "result": {
    "action_taken": "Fixed extendedWaitUntil: changed non-optional wait to optional with correct testTag",
    "files_modified": [".maestro/screenshots/training/training-history.yaml"],
    "ready_for_rerun": true
  },
  "agent_id": "lyra-584",
  "duration_ms": 45230,
  "commentary": "Used training_history_screen testTag from TrainingTestTags.kt. Also fixed analytics tab navigation.",
  "retry_count": 0,
  "protocol_version": "1"
}
```

**Status values:** `"completed"` | `"failed"` | `"rejected"` | `"dead_lettered"`

- `result` — opaque JSON blob. AI-populated. Structure is task-dependent.
- `commentary` — AI-added context beyond the result blob. For human or source-system consumption.
- `retry_count` — how many times the task was re-queued before this resolution. 0 = first attempt succeeded.

### 3.3 SessionContext (Routing + Backpressure)

Broadcast by each AI at session start and when capacity changes significantly (~10% utilization change):

```json
{
  "agent_id": "lyra-584",
  "protocol_version": "1",
  "static_tags": ["android", "fitquest", "maestro", "yaml"],
  "session_tags": ["training", "yaml-fixes"],
  "capacity": {
    "max_pending": 5,
    "priority_floor": "normal",
    "utilization": 0.40,
    "accepting": true,
    "preferred_priority": "normal"
  }
}
```

- `static_tags` — declared capabilities from AI profile. Always true regardless of current session.
- `session_tags` — live context announced at session start or when domain changes. Expire when session ends. Prevents yesterday's context from routing today's tasks.
- `capacity.utilization` — fraction of `max_pending` currently claimed. Smart sources (WebSocket/SSE) throttle when this exceeds ~0.8.
- `capacity.accepting` — if `false`, bus skips this agent entirely during routing.
- `capacity.preferred_priority` — when `utilization > 0.8`, shifts to `"critical"`. Signal to smart sources to voluntarily throttle normal/background tasks.

### 3.4 Error Responses

All error responses share a common shape:

```json
{
  "error": "validation_failed",
  "code": "MISSING_REQUIRED_FIELD",
  "field": "source_id",
  "protocol_version": "1"
}
```

| Code | HTTP Status | Meaning |
|------|-------------|---------|
| `MISSING_REQUIRED_FIELD` | 400 | Required field absent from submission |
| `UNKNOWN_SOURCE` | 401 | `source_id` not registered with this bus |
| `AUTH_FAILED` | 401 | Token invalid or revoked |
| `PROTOCOL_VERSION_MISMATCH` | 400 | See §8 |
| `IDEMPOTENCY_CONFLICT` | 409 | Same `idempotency_key`, different `description` — not a retry, a collision |
| `BACKLOG_FULL` | 503 | Hard global limit reached. Includes `Retry-After` header. |

`IDEMPOTENCY_CONFLICT` is important: if the same key arrives with a different description, it's not a safe dedup — it's ambiguous. Reject loudly. The source has a bug.

### 3.5 Callback Security

The bus server **signs all callback POSTs** with HMAC-SHA256:

```
X-TaskBus-Signature: sha256=<hex_hmac>
```

The signing secret is negotiated at source registration (`teambook bus-register` prints it once). Callback recipients should verify this header before acting on the result. Unsigned or incorrectly-signed callbacks should be rejected.

This prevents arbitrary external systems from injecting fake task results into source systems via the callback URL.

---

## §4 — Routing Logic

*§4 complete. Lyra: tag scoring, open pool, hints vs mandates (4.1–4.3). Cascade PR: added §4.4 claim races, §4.5 task rejection, §4.6 priority escalation.*

### 4.1 Routing by Tags

1. Collect all active `SessionContext` broadcasts (agents with active sessions).
2. For each agent: `score = |intersection(agent.static_tags ∪ agent.session_tags, task.routing_hints.preferred_tags)|`
3. Filter: remove agents where `accepting == false`.
4. Filter: remove agents below `priority_floor` for the task's priority level.
5. Filter: remove agents in `routing_hints.exclude_agents`.
6. Route to highest-scoring agent. Ties broken by `utilization` (prefer less loaded).
7. If no agent matches: add to **open pool**.

### 4.2 Open Pool

Tasks without routing hints, or tasks no agent claims within 60 seconds (configurable), enter the open pool. Any AI can claim from the open pool on standby wake or explicit poll.

`critical` priority tasks in the open pool fire a broadcast wake event to all active agents.

### 4.3 Hints vs Mandates

`preferred_tags` are soft hints, not hard constraints. A task is never stuck because its "preferred" agent is at capacity. If all matching agents are full, task enters open pool rather than waiting indefinitely.

### 4.4 Claim Race Handling

Multiple agents may see a newly-queued task simultaneously (e.g., broadcast wake on critical priority). Claims are **atomic at the daemon level** — the daemon serializes claim requests via the named pipe. First write wins. All other agents receive `{ error: "already_claimed" }` and return to standby or pick from the pool.

The bus server does not need to implement additional locking — the daemon's single-threaded pipe handler is the serialization point.

### 4.5 Task Rejection by AI

An AI that claims a task but determines it is genuinely out of scope (not a failure, a deliberate determination) can **reject** it:

```
RejectTask { task_id, reason: "out of scope — no YAML knowledge in this session" }
```

- Task returns to open pool immediately
- `retry_count` is **not incremented** — rejection is not failure
- Rejection is logged in the event log with `reason`
- If the same AI rejects the same task twice, the bus adds them to `routing_hints.exclude_agents` automatically for that task's remaining lifetime

This distinguishes "AI tried and failed" (timeout → retry_count++) from "AI correctly identified it shouldn't touch this" (rejection → pool, no penalty).

### 4.6 Priority Escalation in Open Pool

Tasks that sit unclaimed in the open pool escalate automatically:

| Priority | Escalation Threshold | Action |
|----------|---------------------|--------|
| `background` | 30 minutes unclaimed | Escalate to `normal` |
| `normal` | 60 minutes unclaimed | Escalate to `critical` + fire broadcast wake |
| `critical` | Never escalates further — already fires wake on entry |

Escalation is recorded in the event log. Source is notified via push/callback when a task escalates (informational, not a result).

This prevents tasks from silently rotting in the pool if all matching agents are offline or busy.

---

## §5 — Reliability

### 5.1 Claim Timeout + Re-queue

Each task has a `claim_timeout_ms` (source-configurable, default 5 min). The timer starts when an AI **claims** the task (not when submitted).

If the claiming AI's session ends or the claim timer expires without resolution:
1. Task returns to the open pool with `retry_count++`
2. If `retry_count < max_retries` (default 3): re-queue at original priority
3. If `retry_count >= max_retries`: dead-letter (§5.2)

### 5.2 Dead-Letter Queue

When a task exhausts retries:

1. `TaskDeadLettered` event written to the permanent event log:
   ```json
   {
     "event": "TaskDeadLettered",
     "task_id": "uuid-v4",
     "idempotency_key": "maestro:water-tracking:run-47",
     "source_id": "maestro-capture",
     "retry_count": 3,
     "last_error": "claim timeout after 3 attempts",
     "timestamp": "2026-02-26T23:04:00Z"
   }
   ```
2. `TaskResult { status: "dead_lettered" }` POSTed to `callback_url` if set, or pushed via WebSocket.
3. Task appears in `teambook tasks --dead-lettered` for human visibility.
4. **Never auto-reinjects.** Source retries at source's discretion.

**Dead-letter rate as health signal:** If N tasks dead-letter within a time window, the bus emits a system health warning. Distinguishes systemic failures from individual task failures.

---

## §6 — Backlog Management

### 6.1 Two-Level Backlog

| Level | Threshold | Action |
|-------|-----------|--------|
| Soft (per-agent) | 80% of `max_pending` | Stop routing `background` priority to this agent. Still accept `critical` / `normal`. |
| Hard (global) | 200 pending tasks total | Reject all new submissions: `503 Service Unavailable` + `Retry-After: N` header. |

### 6.2 Proactive Backpressure

Smart sources (WebSocket/SSE connected) receive updated `SessionContext` when utilization changes by ~10%. When `utilization > 0.8`, `preferred_priority` shifts to `"critical"` — smart sources voluntarily throttle before hitting the hard limit.

This makes the system **self-regulating** rather than cliff-edge. Dumb sources (HTTP POST, one-shot scripts) still get the hard 503. That's their problem to handle.

### 6.3 Per-Source Backlog

Each source has its own backlog cap (daemon config, default 20 pending). When exceeded: **drop-oldest** within that source's tasks. The source's recent state is more relevant than stale queued tasks.

---

## §7 — Authentication & Source Trust

### 7.1 Source Registration

New sources do not auto-attach. Explicit registration:

```bash
teambook bus-register --source maestro --description "Maestro test runner"
# → Generates token, prints it once. Store in source's config.

teambook bus-revoke --source maestro
# → Invalidates token. Source cannot attach until re-registered.

teambook bus-list
# → Shows all registered sources and their status.
```

### 7.2 Token Usage

**WebSocket:** Token in handshake header: `Authorization: Bearer <token>`
**HTTP:** Token in request header: `Authorization: Bearer <token>`

Per-source tokens (not shared). Revocation is clean — drop one source without affecting others.

### 7.3 Connection Identity on Reconnect

When a WebSocket source disconnects and reconnects (network blip, restart):

1. Source presents auth token in new WebSocket handshake
2. Bus server maps `token → source_id`
3. Bus server issues `QueryPendingForSource { source_id }` to daemon via named pipe
4. Daemon queries event log: `ExternalTaskCreated` events with no corresponding `TaskCompleted` for this `source_id`
5. Those tasks are the pending ones — bus server resumes push delivery when they resolve

No new persistence infrastructure. The event log is the source of truth for "what's pending for source X." Token-based is stateless on the client — no session_id to manage or persist.

### 7.4 Audit Trail

Every `ExternalTaskCreated` event in the event log includes `source_id`, agent_id, timestamps, outcome. Full history of what each source injected, who claimed it, what resulted.

---

## §8 — Protocol Versioning

### 8.1 Version Field

Both `TaskSubmission` and `TaskResult` carry `protocol_version: "1"` (major version string).

### 8.2 Compatibility Rules

| Case | Behavior |
|------|----------|
| Unknown fields | Ignored (forward compatibility — v1 clients survive v2 additions) |
| Major version mismatch | Rejected: `{"error": "protocol_version_mismatch", "supported": "1", "received": "2"}` |
| Minor additions | New optional fields only. Always backward compatible. |

Version the protocol from day one. We **will** add fields. Sources compiled against v1 must survive v2 gracefully.

---

## §9 — Event Log vs Task Queue

Two distinct stores with different semantics:

| | Task Queue | Event Log |
|---|---|---|
| **What it is** | Active pending tasks | Permanent audit trail |
| **Consumed?** | Yes — claims remove from queue | No — append-only forever |
| **Replayed?** | Tasks re-enter queue on daemon restart (from log) | Replay shows what happened, doesn't re-inject to external sources |
| **Content** | Full task payload | Thin records: event type, task_id, source_id, agent_id, timestamp, outcome |
| **Lifecycle** | Created → Claimed → Resolved → Gone | `ExternalTaskCreated` → `TaskClaimed` → `TaskCompleted` / `TaskDeadLettered` |

**Restart safety:** On daemon restart, it reads the event log to find tasks in-flight (`ExternalTaskCreated` with no `TaskCompleted`). Those tasks re-enter the queue. The idempotency_key ensures sources don't double-process results they received before the restart.

**Replay semantics:** Replaying the event log for debugging or analytics does NOT re-inject tasks. External source work items existed in external systems that may no longer exist — replaying them would cause havoc. The event log shows what happened; the task queue is what's active.

---

## §10 — Daemon IPC Surface

### 10.1 Current Surface

The daemon's ONLY IPC mechanism is:
- **Windows:** Named Pipes (`\\.\pipe\teamengram`)
- **Linux/Mac:** Unix domain sockets

**There is no HTTP port.** The MCP server communicates with the daemon via pipe, not HTTP.

### 10.2 New Named Pipe Commands

The bus server talks to the daemon via the existing named pipe. New commands to add to the daemon's pipe handler:

```
SubmitExternalTask {
  idempotency_key, source_id, description, metadata,
  priority, routing_hints, claim_timeout_ms
}
→ Response: { task_id: UUID, status: "queued" | "deduplicated" }

QueryPendingForSource {
  source_id
}
→ Response: [{ task_id, idempotency_key, description, created_at, retry_count }]

ResolveExternalTask {
  task_id, status, result, agent_id, commentary
}
→ Response: { ok: true } | { error: "..." }

DeadLetterTask {
  task_id, reason
}
→ Response: { ok: true }
```

The daemon handles these like any other event — emits to event log, wakes AIs via the existing wake system. No new wake reason needed: critical tasks use `WakeReason::Urgent`, normal/background use the existing task queue flow.

---

## §11 — Implementation Order

### Phase 1: Screen Review Connector (Background priority)
Least friction, biggest immediate payoff. Screen review server sends "Capture complete, N screenshots ready" as a background task. AI reviews when free.

Connector: `screen_review_server.py` POSTs to bus on capture completion.

### Phase 2: Maestro Connector (Normal priority)
Maestro flow failure → inject task with YAML path + error. AI fixes. Maestro reruns. Closed loop.

### Phase 3: Gradle Connector (Critical / Normal priority)
Compile errors → normal. Build failures blocking a run → critical.

### Phase 4: GitHub Connector (Normal priority)
PR opened → AI reviews. CI failed → AI investigates.

### Phase 5: AI-to-AI (Cross-cluster, gRPC)
AI-Foundation clusters attach to each other. Task delegation across teams.

---

## §12 — Open Items

- [x] **Cascade:** §3 refinements (max_retries, error responses, callback security) + §4 additions (claim races, task rejection, priority escalation) — complete
- [ ] **Resonance:** Build `bus-test` CLI once §3 schema is finalized — validates any source's integration before it touches production
- [ ] **Implementation:** `task-bus-server` binary. Recommended: `axum` (async, native WebSocket upgrade, SSE support, same Rust ecosystem as daemon)
- [ ] **Daemon:** Add new named pipe commands from §10.2 — low-risk additions to existing pipe handler
- [ ] **QD:** Approve Phase 1 scope before implementation begins

---

*This spec anchors the design discussion from Feb 25–26, 2026. Implementation begins after QD scope approval.*
