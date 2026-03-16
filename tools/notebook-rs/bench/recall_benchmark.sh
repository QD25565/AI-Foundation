#!/usr/bin/env bash
# =============================================================================
# AI-Foundation Notebook Recall Quality Benchmark
# =============================================================================
#
# Measures recall quality BEFORE and AFTER scoring/traversal changes.
# Uses isolated AI_ID=bench-test — never touches real notebooks.
#
# Usage:
#   ./recall_benchmark.sh              # Full suite
#   ./recall_benchmark.sh s1           # Scenario 1 only (basic recall)
#   ./recall_benchmark.sh s2           # Scenario 2 only (recency/updated_at)
#   ./recall_benchmark.sh s3           # Scenario 3 only (BFS graph expansion)
#   ./recall_benchmark.sh clean        # Delete bench notebook and exit
#
# WORKFLOW:
#   1. Run BEFORE your change — record the baseline numbers
#   2. Make your change, rebuild, redeploy
#   3. Run AFTER — compare. If metrics improved, ship. If regressed, investigate.
#
# METRICS:
#   Recall@1  — target note is rank #1
#   Recall@3  — target note appears in top 3
#   MRR       — Mean Reciprocal Rank (1/rank, averaged across queries)
#
# Output format is machine-parseable for CI integration.
# =============================================================================

set -euo pipefail

NOTEBOOK_BIN="${HOME}/.ai-foundation/bin/notebook-cli.exe"
AI_ID="bench-test"

# The notebook binary is a Windows .exe — it uses Windows USERPROFILE for its data path,
# not the WSL $HOME. Resolve the Windows home via wslpath so clean_bench can find the file.
_WIN_HOME=$(wslpath "$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')" 2>/dev/null)
BENCH_DB="${_WIN_HOME:-$HOME}/.ai-foundation/agents/${AI_ID}/notebook.engram"

# Colour output
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; BOLD='\033[1m'; NC='\033[0m'

pass() { echo -e "${GREEN}PASS${NC} $*"; }
fail() { echo -e "${RED}FAIL${NC} $*"; }
info() { echo -e "${BLUE}INFO${NC} $*"; }
header() { echo -e "\n${BOLD}${YELLOW}=== $* ===${NC}"; }

# =============================================================================
# Helpers
# =============================================================================

nb() {
    # WSLENV=AI_ID passes AI_ID through the WSL→Windows env bridge so the
    # Windows binary picks up the isolated bench-test namespace correctly.
    WSLENV=AI_ID AI_ID="$AI_ID" "$NOTEBOOK_BIN" "$@" 2>/dev/null
}

# Save a note, return its ID
save_note() {
    local content="$1"; shift
    local output
    output=$(nb remember "$content" "$@" 2>&1)
    echo "$output" | grep "Note saved: ID" | awk '{print $NF}'
}

# Run recall, return newline-separated IDs in rank order
recall_ids() {
    local query="$1"
    local limit="${2:-10}"
    nb recall "$query" --limit "$limit" 2>&1 \
        | grep -E '^[0-9]+\|' \
        | cut -d'|' -f1
}

# Return rank of target_id in results (1-based), or 0 if not found
rank_of() {
    local target_id="$1"
    local results="$2"
    local rank=0
    local line_num=0
    while IFS= read -r line; do
        line_num=$((line_num + 1))
        if [[ "$line" == "$target_id" ]]; then
            rank=$line_num
            break
        fi
    done <<< "$results"
    echo "$rank"
}

# Accumulate MRR across test cases
# Usage: update_mrr rank
TOTAL_TESTS=0
TOTAL_MRR_NUM=0   # sum of 1/rank (as integer fractions, tracked as integer*1000)
PASS_AT_1=0
PASS_AT_3=0

record_result() {
    local test_name="$1"
    local rank="$2"
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    if [[ "$rank" -eq 0 ]]; then
        fail "$test_name — not found in top-10"
        return
    fi
    if [[ "$rank" -eq 1 ]]; then
        PASS_AT_1=$((PASS_AT_1 + 1))
        PASS_AT_3=$((PASS_AT_3 + 1))
        pass "$test_name — rank #${rank}"
    elif [[ "$rank" -le 3 ]]; then
        PASS_AT_3=$((PASS_AT_3 + 1))
        pass "$test_name — rank #${rank} (in top-3)"
    else
        fail "$test_name — rank #${rank} (outside top-3)"
    fi
    # MRR: track 1000/rank for integer arithmetic, divide at report time
    TOTAL_MRR_NUM=$((TOTAL_MRR_NUM + 1000 / rank))
}

print_summary() {
    header "SUMMARY"
    local r1_pct=$(( PASS_AT_1 * 100 / TOTAL_TESTS ))
    local r3_pct=$(( PASS_AT_3 * 100 / TOTAL_TESTS ))
    local mrr_pct=$(( TOTAL_MRR_NUM / TOTAL_TESTS ))
    echo "Tests run:   ${TOTAL_TESTS}"
    echo "Recall@1:    ${PASS_AT_1}/${TOTAL_TESTS} (${r1_pct}%)"
    echo "Recall@3:    ${PASS_AT_3}/${TOTAL_TESTS} (${r3_pct}%)"
    echo "MRR*1000:    ${mrr_pct}  (ideal=1000, random~100)"
    echo ""
    echo "MACHINE: recall_at_1=${r1_pct} recall_at_3=${r3_pct} mrr=${mrr_pct} tests=${TOTAL_TESTS}"
}

# =============================================================================
# Pre-flight
# =============================================================================

preflight() {
    if [[ ! -f "$NOTEBOOK_BIN" ]]; then
        echo -e "${RED}ERROR${NC}: notebook-cli.exe not found at $NOTEBOOK_BIN"
        exit 1
    fi

    # Smoke-test the binary (catches the batch-delete duplicate alias bug)
    if ! "$NOTEBOOK_BIN" --help &>/dev/null; then
        echo -e "${RED}ERROR${NC}: notebook-cli.exe fails to start. Check for clap alias duplication."
        echo "Known issue: batch-delete command name duplicated as alias. Fix: remove alias=\"batch-delete\" from BatchDelete command."
        exit 1
    fi

    info "Binary OK: $NOTEBOOK_BIN"
    info "AI_ID: $AI_ID  (isolated bench notebook)"
}

clean_bench() {
    if [[ -f "$BENCH_DB" ]]; then
        rm -f "$BENCH_DB"
        info "Cleaned: $BENCH_DB"
    else
        info "Nothing to clean"
    fi
}

# =============================================================================
# SCENARIO 1: Basic Semantic Recall
# =============================================================================
# Tests that keyword+semantic search correctly surfaces relevant notes.
# Should PASS on baseline. If this regresses after any change, something is
# fundamentally broken.
#
# Stability target: Recall@3 = 100% (5/5), Recall@1 >= 60% (3/5)
# =============================================================================

scenario_1() {
    header "S1: Basic Semantic Recall (baseline stability test)"
    clean_bench

    info "Inserting 10 topically distinct notes..."

    ID_RUST=$(save_note  "Rust tokio async runtime event loop concurrent futures" --tags rust,async)
    ID_PYTHON=$(save_note "Python Django web framework REST API views models" --tags python,web)
    ID_ANDROID=$(save_note "Android Kotlin Jetpack Compose UI state management ViewModel" --tags android,kotlin)
    ID_DB=$(save_note "PostgreSQL query optimization indexing EXPLAIN ANALYZE performance" --tags database,sql)
    ID_DOCKER=$(save_note "Docker container orchestration Kubernetes pods deployments" --tags devops,containers)
    ID_ML=$(save_note "Neural network backpropagation training gradient descent loss" --tags ml,ai)
    ID_SEC=$(save_note "TLS certificate authentication OAuth2 JWT bearer tokens" --tags security,auth)
    ID_GIT=$(save_note "Git merge rebase conflict resolution branching strategy" --tags git,vcs)
    ID_PERF=$(save_note "profiling flamegraph allocation heap memory leak detection" --tags performance,debugging)
    ID_ARCH=$(save_note "event sourcing CQRS domain driven design bounded context aggregate" --tags architecture,patterns)

    info "IDs: rust=$ID_RUST python=$ID_PYTHON android=$ID_ANDROID db=$ID_DB docker=$ID_DOCKER"

    # Run recall queries and check ranks
    local results

    results=$(recall_ids "rust async tokio")
    record_result "S1.1 rust async" "$(rank_of "$ID_RUST" "$results")"

    results=$(recall_ids "python web api django")
    record_result "S1.2 python web" "$(rank_of "$ID_PYTHON" "$results")"

    results=$(recall_ids "android compose kotlin ui")
    record_result "S1.3 android compose" "$(rank_of "$ID_ANDROID" "$results")"

    results=$(recall_ids "database sql query performance")
    record_result "S1.4 database sql" "$(rank_of "$ID_DB" "$results")"

    results=$(recall_ids "container kubernetes deployment")
    record_result "S1.5 containers k8s" "$(rank_of "$ID_DOCKER" "$results")"

    results=$(recall_ids "neural network training backpropagation")
    record_result "S1.6 ml neural net" "$(rank_of "$ID_ML" "$results")"

    results=$(recall_ids "authentication jwt oauth token")
    record_result "S1.7 auth security" "$(rank_of "$ID_SEC" "$results")"

    results=$(recall_ids "git merge conflict branching")
    record_result "S1.8 git vcs" "$(rank_of "$ID_GIT" "$results")"

    results=$(recall_ids "memory profiling performance heap")
    record_result "S1.9 profiling perf" "$(rank_of "$ID_PERF" "$results")"

    results=$(recall_ids "event sourcing domain driven design cqrs")
    record_result "S1.10 architecture ddd" "$(rank_of "$ID_ARCH" "$results")"
}

# =============================================================================
# SCENARIO 2: Recency via updated_at
# =============================================================================
# Tests that a recently UPDATED note ranks above one that was saved earlier
# but never touched since.
#
# BEFORE Sage's change (updated_at field): expect FAIL — ties broken arbitrarily.
# AFTER Sage's change: expect PASS — updated note ranks #1 consistently.
#
# Method: Save note_OLD, then note_NEW with similar content. Update note_OLD
# (adds new updated_at >> note_NEW's creation). Query the shared topic.
# note_OLD should now win on recency.
# =============================================================================

scenario_2() {
    header "S2: Recency via updated_at (tests Sage's change)"
    clean_bench

    info "Inserting two similar-content notes..."
    ID_OLD=$(save_note "gradient descent machine learning optimisation loss function convergence" --tags ml,optimization)
    sleep 1  # Ensure ID_NEW has a later timestamp
    ID_NEW=$(save_note "gradient descent optimiser loss convergence rate learning schedule" --tags ml,optimization)

    info "  ID_OLD=$ID_OLD (saved first, will be updated)"
    info "  ID_NEW=$ID_NEW (saved after, never updated)"

    sleep 1
    info "Updating ID_OLD to bump its updated_at..."
    nb update "$ID_OLD" --content "gradient descent machine learning optimisation loss function convergence — UPDATED with Adam, RMSprop, momentum variants" 2>/dev/null || true

    local results
    results=$(recall_ids "gradient descent machine learning optimization")
    local rank_old rank_new
    rank_old=$(rank_of "$ID_OLD" "$results")
    rank_new=$(rank_of "$ID_NEW" "$results")

    info "  ID_OLD rank: ${rank_old} (should be 1 after recency fix)"
    info "  ID_NEW rank: ${rank_new}"

    if [[ "$rank_old" -eq 1 ]]; then
        record_result "S2.1 updated note ranks #1" "1"
    elif [[ "$rank_old" -gt 0 && "$rank_old" -lt "$rank_new" ]]; then
        record_result "S2.1 updated note ranks above unmodified" "$rank_old"
    else
        record_result "S2.1 updated note ranks above unmodified" "0"
    fi

    # Cross-check: the never-updated note should NOT be #1
    if [[ "$rank_new" -ne 1 ]]; then
        record_result "S2.2 unmodified note not #1" "1"
    else
        record_result "S2.2 unmodified note not #1" "0"
    fi
}

# =============================================================================
# SCENARIO 3: BFS Graph Expansion
# =============================================================================
# Tests that multi-hop graph traversal at recall time surfaces notes that
# are NOT directly linked to query-matching seed notes.
#
# BEFORE Vesper's change: note_C absent from top-5 (not directly linked to A)
# AFTER Vesper's change: note_C surfaces (2 hops: A→B→C)
#
# Setup: A→B linked, B→C linked. A→C NOT directly linked.
# Query matches A. Expect C to appear in expanded results.
# =============================================================================

scenario_3() {
    header "S3: BFS Graph Expansion (tests Vesper's change)"
    clean_bench

    info "Inserting three notes for graph traversal test..."
    ID_A=$(save_note "neural network architecture layers activation functions deep learning" --tags ml,dl)
    ID_B=$(save_note "backpropagation weight update gradient chain rule calculus" --tags ml,math)
    ID_C=$(save_note "stochastic gradient descent Adam momentum optimizer convergence rate" --tags ml,optimizer)

    info "  A=$ID_A (neural network — directly queryable)"
    info "  B=$ID_B (backprop — linked to A)"
    info "  C=$ID_C (optimizer — linked to B, NOT to A)"

    info "Creating edges: A→B, B→C (no A→C direct link)..."
    nb link "$ID_A" "$ID_B" 2>/dev/null || true
    nb link "$ID_B" "$ID_C" 2>/dev/null || true

    local results
    results=$(recall_ids "neural network deep learning architecture" 10)
    local rank_a rank_b rank_c
    rank_a=$(rank_of "$ID_A" "$results")
    rank_b=$(rank_of "$ID_B" "$results")
    rank_c=$(rank_of "$ID_C" "$results")

    info "  A rank: ${rank_a} (expect #1 — direct match)"
    info "  B rank: ${rank_b} (expect in top-5 — 1 hop)"
    info "  C rank: ${rank_c} (0 before BFS, in top-10 after)"

    record_result "S3.1 direct match (A) in top-3" "$rank_a"
    if [[ "$rank_b" -gt 0 && "$rank_b" -le 5 ]]; then
        record_result "S3.2 1-hop node (B) in top-5" "1"
    else
        record_result "S3.2 1-hop node (B) in top-5" "0"
        info "  NOTE: S3.2 EXPECTED to fail before Vesper's BFS change. Fail = baseline."
    fi

    # This is the key BFS test — C is 2 hops away
    if [[ "$rank_c" -gt 0 && "$rank_c" -le 10 ]]; then
        record_result "S3.3 2-hop node (C) surfaces in top-10 [BFS]" "1"
    else
        record_result "S3.3 2-hop node (C) surfaces in top-10 [BFS]" "0"
        info "  NOTE: S3.3 EXPECTED to fail before Vesper's BFS change. Fail = baseline."
    fi
}

# =============================================================================
# SCENARIO 4: Temporal Edge Invalidation
# =============================================================================
# Tests that a graph edge marked invalid (t_invalid set) is excluded from
# graph-based scoring, so the linked note no longer surfaces via that edge.
#
# BEFORE Lyra's edge schema + Sage's storage.rs fix: FAIL — outgoing_edges()
# returns all edges regardless of t_invalid; stale note still surfaces.
# AFTER both changes: PASS — valid_outgoing_edges(now) filters out the stale
# edge; note_B has no direct match and disappears from results.
#
# SKIP: if `notebook invalidate-edge` CLI command is not yet deployed.
#
# Setup:
#   note_A — strong semantic match to query (direct hit)
#   note_B — content that would NOT rank for the query on its own
#   Edge A→B (explicit link, so B surfaces via graph from A)
#
# Steps:
#   1. Recall query → note_B in top-10 (via graph edge from A)    [S4.1]
#   2. Invalidate A→B
#   3. Recall same query → note_B absent from top-10             [S4.2]
# =============================================================================

scenario_4() {
    header "S4: Temporal Edge Invalidation (tests Lyra's t_invalid + Sage's storage.rs)"
    clean_bench

    # Check CLI command is available before running
    if ! nb invalidate-edge --help &>/dev/null 2>&1; then
        echo -e "${YELLOW}SKIP${NC} S4: 'notebook invalidate-edge' command not yet deployed."
        echo "       Waiting on notebook-cli.rs to expose CsrGraph::invalidate_edge()."
        return
    fi

    info "Inserting two notes — one strong query match, one off-topic..."
    ID_A=$(save_note "Rust async tokio runtime executor future poll waker" --tags rust,async)
    ID_B=$(save_note "legacy synchronous blocking IO thread-per-connection Apache prefork" --tags legacy,networking)

    info "  ID_A=$ID_A (Rust async — direct match)"
    info "  ID_B=$ID_B (legacy blocking IO — should NOT match query directly)"

    info "Creating edge A→B..."
    nb link "$ID_A" "$ID_B" 2>/dev/null || true

    local results rank_b_before rank_b_after

    # Step 1: B should surface via graph edge from A
    results=$(recall_ids "rust async tokio executor" 10)
    rank_b_before=$(rank_of "$ID_B" "$results")
    info "  Before invalidation: ID_B rank = ${rank_b_before} (expect in top-10 via graph)"

    if [[ "$rank_b_before" -gt 0 && "$rank_b_before" -le 10 ]]; then
        record_result "S4.1 linked note surfaces via graph edge" "1"
    else
        record_result "S4.1 linked note surfaces via graph edge" "0"
        info "  NOTE: S4.1 EXPECTED to fail before Vesper's BFS change. Edge exists but graph expansion not yet active."
    fi

    # Step 2: Invalidate the edge
    info "Invalidating edge A→B..."
    nb invalidate-edge "$ID_A" "$ID_B" 2>/dev/null || true

    # Step 3: B should no longer surface (no direct match, edge invalid)
    results=$(recall_ids "rust async tokio executor" 10)
    rank_b_after=$(rank_of "$ID_B" "$results")
    info "  After invalidation: ID_B rank = ${rank_b_after} (expect 0 — absent)"

    if [[ "$rank_b_after" -eq 0 ]]; then
        record_result "S4.2 invalidated note absent from top-10" "1"
    else
        record_result "S4.2 invalidated note absent from top-10" "0"
        info "  NOTE: S4.2 EXPECTED to fail before Sage's storage.rs fix (valid_outgoing_edges swap)."
    fi
}

# =============================================================================
# Main
# =============================================================================

main() {
    local scenario="${1:-all}"

    echo -e "${BOLD}AI-Foundation Notebook Recall Benchmark${NC}"
    echo "Timestamp: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "Binary:    $NOTEBOOK_BIN"
    echo "AI_ID:     $AI_ID"
    echo ""

    if [[ "$scenario" == "clean" ]]; then
        clean_bench
        exit 0
    fi

    preflight

    case "$scenario" in
        s1) scenario_1 ;;
        s2) scenario_2 ;;
        s3) scenario_3 ;;
        s4) scenario_4 ;;
        all)
            scenario_1
            scenario_2
            scenario_3
            scenario_4
            ;;
        *)
            echo "Unknown scenario: $scenario. Use s1, s2, s3, s4, all, or clean."
            exit 1
            ;;
    esac

    print_summary
    clean_bench
}

main "${1:-all}"
