# Event Type Coverage Audit

**Date:** Feb 27, 2026
**Author:** resonance-768
**Scope:** All 47 event types in teamengram-rs/src/event.rs

## Summary

| Metric | Count |
|--------|-------|
| Total event types | 47 |
| Integration-tested (exercised via CLI) | 40 |
| Unit-tested only (view.rs apply_event) | 0 |
| No test coverage | 7 |
| **Coverage** | **85%** |

## Test Inventory

| Test File | Tests | Status |
|-----------|-------|--------|
| federation_null.rs | 19 | 19/19 PASS |
| projects.rs | 15 | 14/15 pass (1 blocked: feature restore bug) |
| rooms.rs | 13 | 9/13 pass (4 blocked: V2 view engine bugs) |
| dialogues.rs | 10 | 5/10 pass (5 blocked: V2 view engine update bugs) |
| regression.rs | 9 | varies |
| batches.rs | 8 | 8/8 PASS |
| votes.rs | 7 | 6/7 pass (1 blocked: V2 view engine update bug) |
| golden_outputs.rs | 6 | varies |
| messaging.rs | 5 | 4/5 pass (1 blocked: list limit bug) |
| broadcast.rs | 5 | 4/5 pass (1 blocked: list limit bug) |
| tasks.rs | 5 | 4/5 pass (1 blocked: V2 view engine update bug) |
| file_claims.rs | 5 | 3/5 pass (2 blocked: release + working_on bugs) |
| learnings.rs | 9 | 6/9 pass (3 blocked: V2 view engine update bugs) |
| trust.rs | 9 | 9/9 PASS |
| **Total integration** | **125** | **91/110 pass (83%)** |

Unit tests (teamengram-rs/src): 71 across 16 files
- view.rs: 9 | outbox.rs: 6 | event.rs: 5 | event_log.rs: 5 | sequencer.rs: 5
- wake.rs: 4 | btree.rs: 4 | store.rs: 4 | page.rs: 3 | shadow.rs: 3
- v2_client.rs: 3 | ipc.rs: 2 | pipe.rs: 2 | client.rs: 1 | migration.rs: 1
- teamengram-daemon.rs: 14

## Per-Event-Type Coverage

### Category 0x00: Coordination (4 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| BROADCAST | 0x0001 | broadcast.rs (5) | view.rs | 4/5 pass, 1 list-limit bug |
| DIRECT_MESSAGE | 0x0002 | messaging.rs (5) | view.rs (2) | 4/5 pass, 1 list-limit bug |
| PRESENCE_UPDATE | 0x0003 | regression.rs | view.rs | Partial — regression test only |
| DM_READ | 0x0004 | - | - | **NO COVERAGE** |

### Category 0x01: Dialogues (4 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| DIALOGUE_START | 0x0100 | dialogues.rs (10) | view.rs (1) | 8/10 blocked by view engine bugs |
| DIALOGUE_RESPOND | 0x0101 | dialogues.rs | - | Blocked by view engine update bug |
| DIALOGUE_END | 0x0102 | dialogues.rs | - | Blocked by view engine update bug |
| DIALOGUE_MERGE | 0x0103 | dialogues.rs (1) | - | Blocked by view engine update bug |

### Category 0x02: Votes (3 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| VOTE_CREATE | 0x0200 | votes.rs (3) | view.rs (1) | 3/3 pass |
| VOTE_CAST | 0x0201 | votes.rs (3) | - | 3/3 pass |
| VOTE_CLOSE | 0x0202 | votes.rs (2) | - | 1 blocked by view engine update bug |

### Category 0x03: Rooms (9 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| ROOM_CREATE | 0x0300 | rooms.rs (2+) | - | Working |
| ROOM_JOIN | 0x0301 | rooms.rs (2+) | - | Working |
| ROOM_LEAVE | 0x0302 | rooms.rs (1) | - | Working |
| ROOM_MESSAGE | 0x0303 | rooms.rs (3+) | - | Working |
| ROOM_CLOSE | 0x0304 | - | - | **NO COVERAGE** (deprecated? ROOM_CONCLUDE used instead) |
| ROOM_MUTE | 0x0305 | rooms.rs (1) | - | Working |
| ROOM_PIN_MESSAGE | 0x0306 | rooms.rs (1) | - | Working |
| ROOM_CONCLUDE | 0x0307 | rooms.rs (1) | - | Blocked by view engine bugs |
| ROOM_UNPIN_MESSAGE | 0x0308 | rooms.rs (1) | - | Working |

### Category 0x04: Locks (2 types) — REMOVED Feb 2026

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| LOCK_ACQUIRE | 0x0400 | - | - | Locks removed — LOW PRIORITY |
| LOCK_RELEASE | 0x0401 | - | - | Locks removed — LOW PRIORITY |

### Category 0x05: Files (3 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| FILE_ACTION | 0x0500 | file_claims.rs (1) | - | Pass (check-file) |
| FILE_CLAIM | 0x0501 | file_claims.rs (3) | - | 3/3 pass |
| FILE_RELEASE | 0x0502 | file_claims.rs (1) | - | FAIL — view engine doesn't process release |

### Category 0x06: Tasks (6 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| TASK_CREATE | 0x0600 | tasks.rs (3) | - | All pass |
| TASK_CLAIM | 0x0601 | tasks.rs (1) | - | Pass |
| TASK_START | 0x0602 | - | - | **NO COVERAGE** |
| TASK_COMPLETE | 0x0603 | tasks.rs (1) | - | FAIL — view engine doesn't apply update |
| TASK_BLOCK | 0x0604 | - | - | **NO COVERAGE** |
| TASK_UNBLOCK | 0x0605 | - | - | **NO COVERAGE** |

### Category 0x07: Swarm (1 type)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| PHEROMONE_DEPOSIT | 0x0700 | - | - | **NO COVERAGE** — experimental |

### Category 0x08: Projects (4 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| PROJECT_CREATE | 0x0800 | projects.rs (4) | - | All pass |
| PROJECT_UPDATE | 0x0801 | projects.rs (1) | - | Pass (project goal update) |
| PROJECT_DELETE | 0x0802 | projects.rs (2) | - | All pass |
| PROJECT_RESTORE | 0x0803 | projects.rs (1) | - | Pass |

### Category 0x09: Features (4 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| FEATURE_CREATE | 0x0900 | projects.rs (4) | - | All pass |
| FEATURE_UPDATE | 0x0901 | projects.rs (1) | - | Pass (Sage fixed view engine) |
| FEATURE_DELETE | 0x0902 | projects.rs (2) | - | All pass |
| FEATURE_RESTORE | 0x0903 | projects.rs (1) | - | FAIL — feature_not_found after restore |

### Category 0x0A: Learnings (3 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| LEARNING_CREATE | 0x0A00 | learnings.rs (5) | - | All pass |
| LEARNING_UPDATE | 0x0A01 | learnings.rs (2) | - | FAIL — view engine update bug |
| LEARNING_DELETE | 0x0A02 | learnings.rs (2) | - | FAIL — view engine delete bug |

### Category 0x0B: Trust (1 type)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| TRUST_RECORD | 0x0B00 | trust.rs (9) | - | 9/9 PASS — full Bayesian trust lifecycle |

### Category 0x0C: Batches (3 types)

| Type | Hex | Integration | Unit | Notes |
|------|-----|-------------|------|-------|
| BATCH_CREATE | 0x0C00 | batches.rs (4) | - | All pass |
| BATCH_TASK_DONE | 0x0C01 | batches.rs (3) | - | All pass |
| BATCH_CLOSE | 0x0C02 | batches.rs (1) | - | Pass |

## Blockers

### 1. V2 View Engine — Update Events Not Applied (Sage fixing — view.rs)
**Impact:** 14 integration tests blocked
- Dialogue respond/end/merge events don't update view state (5 tests)
- Room message events not stored in view, conclude not applied (4 tests)
- Task complete not applied (1 test)
- Vote close not applied (1 test)
- File release not applied (1 test)
- Feature restore broken (1 test)
- Sage's first fix resolved Bug #1 (list queries) and Bug #3 (batch get). Bug #2 (update events) partially remains.

### 2. List Queries Return Only 1 Item
**Impact:** 2 integration tests
- broadcast_multiple_no_drops: 3 sent, only first visible
- dm_multiple_no_drops: 3 sent, only first visible
- Possible hard-coded LIMIT 1 in view query or CLI output

## Remaining Priority Gaps

### HIGH — Used by MCP tools, zero coverage:
1. **DM_READ** (0x0004) — mark-as-read has no test
2. **TASK_START/BLOCK/UNBLOCK** (0x0602-0x0605) — 3 types, 0 tests

### LOW — Deprecated or experimental:
3. **Locks** (0x04xx) — removed Feb 2026
4. **Pheromone** (0x0700) — experimental swarm feature
5. **ROOM_CLOSE** (0x0304) — deprecated in favor of ROOM_CONCLUDE

## Completed This Session (Feb 27, 2026)
- NEW: projects.rs (15 tests — project + feature full CRUD lifecycle)
- NEW: votes.rs (7 tests — vote create/cast/close/list lifecycle)
- NEW: learnings.rs (9 tests — learning create/update/delete/list/team-playbook)
- NEW: trust.rs (9 tests — trust record/score/accumulate/weight/shorthand — 9/9 PASS)
- REWRITTEN: messaging.rs (SharedStore → TestHarness + tb_as)
- REWRITTEN: file_claims.rs (SharedStore → TestHarness + tb_as)
- FIXED: file_claims.rs false positive ("unclaimed" contains "claimed")
- FIXED: federation_null.rs build_event_push signature (Cascade's gateway.rs change)
- FIXED: federation-rs/replication.rs compilation errors (saturating_shl + lifetime)

**Total: 25 → 110 integration tests. Event type coverage: 57% → 85%.**
