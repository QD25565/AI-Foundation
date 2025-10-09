#!/usr/bin/env python3
"""
TEAMBOOK EVOLUTION v2.0.0 - COLLABORATIVE PROBLEM SOLVING
===========================================================
Multiple AIs work together on problems. Submit ideas, rank them, combine best parts.

Simple for AIs:
- evolve(goal) - Start working on a problem
- contribute(evo_id, content) - Share your approach
- contributions(evo_id) - See all ideas (ranked)
- synthesize(evo_id) - Combine best ideas into solution

Security: Size limits, rate limits, SQL injection prevention
Performance: Cached scores, indexed queries, batch operations
"""

import time
import re
from datetime import datetime, timedelta, timezone
from typing import Dict, List, Optional, Any, Tuple
from collections import defaultdict
import logging
import json

from teambook_shared import (
    CURRENT_AI_ID, CURRENT_TEAMBOOK, OUTPUT_FORMAT,
    pipe_escape, format_time_compact, clean_text, simple_summary
)

from teambook_storage import get_db_conn, log_operation_to_db

# ============= SECURITY LIMITS =============

MAX_CONTRIBUTIONS_PER_AI = 10  # Per evolution
MAX_EVOLUTIONS_PER_TEAMBOOK = 20  # Concurrent
MAX_CONTRIBUTION_LENGTH = 10000  # 10KB per contribution
MAX_SYNTHESIS_PER_HOUR = 10  # Rate limit synthesis
MAX_VOTE_CHANGES = 5  # Can change vote 5 times max

# Rate limiting
_synthesis_limiter = defaultdict(list)  # teambook -> [timestamps]

# Score cache
_score_cache = {}  # contrib_id -> (score, timestamp)
_cache_ttl = 300  # 5 minutes

# ============= INPUT VALIDATION =============

def sanitize_approach(approach: str) -> Optional[str]:
    """Sanitize approach label"""
    if not approach:
        return None

    approach = str(approach).strip().lower()

    if len(approach) > 100:
        approach = approach[:100]

    # Remove special chars
    approach = re.sub(r'[^a-z0-9\s\-]', '', approach)

    return approach if approach else None

def check_synthesis_rate(teambook: str) -> Tuple[bool, int]:
    """Check synthesis rate limit"""
    now = time.time()
    hour_ago = now - 3600

    _synthesis_limiter[teambook] = [t for t in _synthesis_limiter[teambook] if t > hour_ago]

    current = len(_synthesis_limiter[teambook])
    remaining = MAX_SYNTHESIS_PER_HOUR - current

    if current >= MAX_SYNTHESIS_PER_HOUR:
        return False, 0

    _synthesis_limiter[teambook].append(now)
    return True, remaining - 1

# ============= DATABASE INITIALIZATION =============

def init_evolution_tables(conn):
    """Initialize enhanced evolution tables"""

    # Create sequences for auto-increment
    try:
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_contributions')
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_conflicts')
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_synthesis')
    except Exception:
        pass  # Sequences might already exist

    # Contributions table (replaces old attempts)
    conn.execute('''
        CREATE TABLE IF NOT EXISTS contributions (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_contributions'),
            evo_id INTEGER NOT NULL,
            author_ai_id VARCHAR(100) NOT NULL,
            content TEXT NOT NULL,
            approach VARCHAR(100),
            created_at TIMESTAMPTZ NOT NULL,
            avg_score REAL DEFAULT 0.0,
            rank_count INTEGER DEFAULT 0,
            teambook_name VARCHAR(50)
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_contrib_evo ON contributions(evo_id, avg_score DESC)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_contrib_author ON contributions(author_ai_id)')

    # Rankings table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS contribution_rankings (
            contrib_id INTEGER NOT NULL,
            ranker_ai_id VARCHAR(100) NOT NULL,
            score REAL NOT NULL,
            reason TEXT,
            created_at TIMESTAMPTZ NOT NULL,
            PRIMARY KEY(contrib_id, ranker_ai_id)
        )
    ''')

    # Conflicts table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS contribution_conflicts (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_conflicts'),
            evo_id INTEGER NOT NULL,
            contrib_ids TEXT NOT NULL,
            conflict_type VARCHAR(50),
            severity VARCHAR(20),
            description TEXT,
            resolved BOOLEAN DEFAULT FALSE
        )
    ''')

    # Votes table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS contribution_votes (
            evo_id INTEGER NOT NULL,
            voter_ai_id VARCHAR(100) NOT NULL,
            preferences TEXT NOT NULL,
            vote_changes INTEGER DEFAULT 0,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ,
            PRIMARY KEY(evo_id, voter_ai_id)
        )
    ''')

    # Synthesis history
    conn.execute('''
        CREATE TABLE IF NOT EXISTS synthesis_history (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_synthesis'),
            evo_id INTEGER NOT NULL,
            strategy VARCHAR(50) NOT NULL,
            contrib_ids TEXT NOT NULL,
            conflicts_detected INTEGER DEFAULT 0,
            output_path TEXT,
            created_at TIMESTAMPTZ NOT NULL,
            created_by VARCHAR(100) NOT NULL
        )
    ''')

    conn.commit()

# ============= CORE FUNCTIONS =============

def evolve(goal: str = None, output: str = None, **kwargs) -> Dict:
    """Start working on a problem"""
    try:
        goal = str(kwargs.get('goal', goal) or '').strip()
        output_file = str(kwargs.get('output', output) or '').strip()

        if not goal:
            return {"error": "goal_required"}

        goal = clean_text(goal)[:500]  # Limit goal length

        if not output_file:
            # Auto-generate filename from goal
            safe_goal = re.sub(r'[^a-z0-9_\-]', '', goal.lower()[:30])
            output_file = f"{safe_goal}_{int(time.time())}.txt"

        with get_db_conn() as conn:
            init_evolution_tables(conn)

            # Check evolution limit
            active_count = conn.execute(
                "SELECT COUNT(*) FROM notes WHERE type = 'evolution' AND teambook_name = ?",
                [CURRENT_TEAMBOOK]
            ).fetchone()[0]

            if active_count >= MAX_EVOLUTIONS_PER_TEAMBOOK:
                return {"error": f"evolution_limit|max:{MAX_EVOLUTIONS_PER_TEAMBOOK}"}

            # Create evolution note
            max_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM notes").fetchone()[0]
            evo_id = max_id + 1

            conn.execute('''
                INSERT INTO notes (
                    id, content, summary, type, author, owner,
                    teambook_name, created, pinned
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ''', [
                evo_id,
                f"EVOLUTION: {goal}\nOutput: {output_file}",
                f"Evolution: {goal[:100]}",
                "evolution",
                CURRENT_AI_ID,
                None,  # No owner - collaborative
                CURRENT_TEAMBOOK,
                datetime.now(timezone.utc),
                False
            ])

        log_operation_to_db('evolve')

        result = f"evo:{evo_id}|{output_file}"
        return {"evolution": result}

    except Exception as e:
        logging.error(f"Evolve error: {e}")
        return {"error": "evolve_failed"}

def contribute(evo_id: Any = None, content: str = None, approach: str = None, **kwargs) -> Dict:
    """Share your approach to a problem"""
    try:
        # Security: Use centralized ID normalization
        from teambook_shared import get_note_id
        evo_id = kwargs.get('evo_id', evo_id)
        evo_id = get_note_id(evo_id)

        if not evo_id:
            return {"error": "evo_id_required"}

        content = str(kwargs.get('content', content) or '').strip()
        if not content:
            return {"error": "content_required"}

        # Sanitize
        content = clean_text(content)
        truncated = False
        if len(content) > MAX_CONTRIBUTION_LENGTH:
            content = content[:MAX_CONTRIBUTION_LENGTH]
            truncated = True

        approach = sanitize_approach(kwargs.get('approach', approach))

        with get_db_conn() as conn:
            init_evolution_tables(conn)

            # Verify evolution exists
            evo = conn.execute(
                "SELECT id FROM notes WHERE id = ? AND type = 'evolution'",
                [evo_id]
            ).fetchone()

            if not evo:
                return {"error": "evolution_not_found"}

            # Check contribution limit for this AI
            contrib_count = conn.execute(
                "SELECT COUNT(*) FROM contributions WHERE evo_id = ? AND author_ai_id = ?",
                [evo_id, CURRENT_AI_ID]
            ).fetchone()[0]

            if contrib_count >= MAX_CONTRIBUTIONS_PER_AI:
                return {"error": f"contribution_limit|max:{MAX_CONTRIBUTIONS_PER_AI}"}

            # Insert contribution
            cursor = conn.execute('''
                INSERT INTO contributions (
                    evo_id, author_ai_id, content, approach,
                    created_at, teambook_name
                ) VALUES (?, ?, ?, ?, ?, ?)
                RETURNING id
            ''', [
                evo_id, CURRENT_AI_ID, content, approach,
                datetime.now(timezone.utc), CURRENT_TEAMBOOK
            ])

            contrib_id = cursor.fetchone()[0]

            # Emit event (if event system available)
            try:
                from teambook_events import emit_event
                emit_event('evolution', str(evo_id), 'contributed',
                          f"{CURRENT_AI_ID} contributed" + (f" ({approach})" if approach else ""))
            except ImportError:
                pass

        log_operation_to_db('contribute')

        result = f"contrib:{contrib_id}|evo:{evo_id}"
        if approach:
            result += f"|{approach}"
        if truncated:
            result += "|truncated"

        return {"contributed": result}

    except Exception as e:
        logging.error(f"Contribute error: {e}")
        return {"error": "contribute_failed"}

def rank_contribution(contrib_id: int = None, score: float = None, reason: str = None, **kwargs) -> Dict:
    """Rate an idea (0-10)"""
    try:
        contrib_id = int(kwargs.get('contrib_id', contrib_id))
        score = float(kwargs.get('score', score))
        reason = kwargs.get('reason', reason)

        if not contrib_id:
            return {"error": "contrib_id_required"}

        # Validate score
        if score < 0 or score > 10:
            return {"error": "score_range|0-10"}

        # Sanitize reason
        if reason:
            reason = clean_text(reason)[:200]

        with get_db_conn() as conn:
            init_evolution_tables(conn)

            # Verify contribution exists
            contrib = conn.execute(
                "SELECT author_ai_id, evo_id FROM contributions WHERE id = ?",
                [contrib_id]
            ).fetchone()

            if not contrib:
                return {"error": "contribution_not_found"}

            author, evo_id = contrib

            # Insert or update ranking
            conn.execute('''
                INSERT INTO contribution_rankings (contrib_id, ranker_ai_id, score, reason, created_at)
                VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(contrib_id, ranker_ai_id) DO UPDATE SET
                    score = excluded.score,
                    reason = excluded.reason,
                    created_at = excluded.created_at
            ''', [contrib_id, CURRENT_AI_ID, score, reason, datetime.now(timezone.utc)])

            # Recalculate average score
            avg_result = conn.execute('''
                SELECT AVG(score), COUNT(*) FROM contribution_rankings
                WHERE contrib_id = ?
            ''', [contrib_id]).fetchone()

            avg_score, rank_count = avg_result

            # Update contribution
            conn.execute('''
                UPDATE contributions
                SET avg_score = ?, rank_count = ?
                WHERE id = ?
            ''', [avg_score, rank_count, contrib_id])

            # Invalidate cache
            _score_cache.pop(contrib_id, None)

            # Emit event
            try:
                from teambook_events import emit_event
                emit_event('contribution', str(contrib_id), 'ranked',
                          f"Ranked {score}/10 (avg: {avg_score:.1f})")
            except ImportError:
                pass

        log_operation_to_db('rank_contribution')

        result = f"contrib:{contrib_id}|score:{score}|avg:{avg_score:.1f}"
        return {"ranked": result}

    except Exception as e:
        logging.error(f"Rank error: {e}")
        return {"error": "rank_failed"}

def contributions(evo_id: Any = None, sort: str = "ranked", **kwargs) -> Dict:
    """See all ideas (ranked by score)"""
    try:
        # Security: Use centralized ID normalization
        from teambook_shared import get_note_id
        evo_id = kwargs.get('evo_id', evo_id)
        evo_id = get_note_id(evo_id)

        if not evo_id:
            return {"error": "evo_id_required"}

        sort = str(kwargs.get('sort', sort or 'ranked')).lower()

        # Determine sort order
        if sort == 'recent':
            order = 'created_at DESC'
        elif sort == 'author':
            order = 'author_ai_id, created_at DESC'
        else:  # 'ranked' (default)
            order = 'avg_score DESC, rank_count DESC, created_at DESC'

        with get_db_conn() as conn:
            init_evolution_tables(conn)

            contribs = conn.execute(f'''
                SELECT id, author_ai_id, content, approach, avg_score, rank_count, created_at
                FROM contributions
                WHERE evo_id = ?
                ORDER BY {order}
            ''', [evo_id]).fetchall()

        if not contribs:
            return {"msg": "no_contributions"}

        log_operation_to_db('contributions')

        if OUTPUT_FORMAT == 'pipe':
            lines = []
            for cid, author, content, approach, score, ranks, created in contribs:
                parts = [
                    f"contrib:{cid}",
                    author,
                    f"{score:.1f}" if score else "unranked",
                    format_time_compact(created)
                ]
                if approach:
                    parts.insert(2, approach)

                # Add content preview (first 50 chars)
                preview = content[:50].replace('\n', ' ')
                parts.append(preview)

                lines.append('|'.join(pipe_escape(p) for p in parts))

            return {"contributions": lines}
        else:
            formatted = []
            for cid, author, content, approach, score, ranks, created in contribs:
                item = {
                    'id': cid,
                    'author': author,
                    'score': round(score, 1) if score else None,
                    'rankings': ranks,
                    'time': format_time_compact(created),
                    'preview': content[:100]
                }
                if approach:
                    item['approach'] = approach
                formatted.append(item)

            return {"contributions": formatted}

    except Exception as e:
        logging.error(f"Contributions error: {e}")
        return {"error": "contributions_failed"}

def synthesize(evo_id: Any = None, strategy: str = "top", min_score: float = 7.0, **kwargs) -> Dict:
    """Combine best ideas into solution"""
    try:
        # Rate limiting
        allowed, remaining = check_synthesis_rate(CURRENT_TEAMBOOK or "private")
        if not allowed:
            return {"error": "synthesis_limit|wait_1h"}

        # Security: Use centralized ID normalization
        from teambook_shared import get_note_id
        evo_id = kwargs.get('evo_id', evo_id)
        evo_id = get_note_id(evo_id)

        if not evo_id:
            return {"error": "evo_id_required"}

        strategy = str(kwargs.get('strategy', strategy or 'top')).lower()
        min_score = float(kwargs.get('min_score', min_score or 7.0))

        with get_db_conn() as conn:
            init_evolution_tables(conn)

            # Get evolution info
            evo = conn.execute('''
                SELECT content FROM notes
                WHERE id = ? AND type = 'evolution'
            ''', [evo_id]).fetchone()

            if not evo:
                return {"error": "evolution_not_found"}

            # Extract output filename
            output_file = "output.txt"
            for line in evo[0].split('\n'):
                if line.startswith('Output:'):
                    output_file = line.split(':', 1)[1].strip()
                    break

            # Get contributions based on strategy
            if strategy == 'all':
                query = '''
                    SELECT id, content, author_ai_id, avg_score
                    FROM contributions
                    WHERE evo_id = ?
                    ORDER BY created_at
                '''
                params = [evo_id]
            elif strategy == 'consensus':
                query = '''
                    SELECT id, content, author_ai_id, avg_score
                    FROM contributions
                    WHERE evo_id = ? AND avg_score >= 9.0
                    ORDER BY avg_score DESC
                '''
                params = [evo_id]
            else:  # 'top' (default)
                query = '''
                    SELECT id, content, author_ai_id, avg_score
                    FROM contributions
                    WHERE evo_id = ? AND avg_score >= ?
                    ORDER BY avg_score DESC, rank_count DESC
                    LIMIT 5
                '''
                params = [evo_id, min_score]

            contribs = conn.execute(query, params).fetchall()

            if not contribs:
                return {"error": "no_qualified_contributions"}

            # Synthesize content
            output_lines = []
            output_lines.append(f"# Synthesis of Evolution {evo_id}")
            output_lines.append(f"# Strategy: {strategy}, Min Score: {min_score}")
            output_lines.append(f"# Combined {len(contribs)} contributions")
            output_lines.append("")

            for cid, content, author, score in contribs:
                output_lines.append(f"## Contribution {cid} by {author} (Score: {score:.1f})")
                output_lines.append("")
                output_lines.append(content)
                output_lines.append("")
                output_lines.append("---")
                output_lines.append("")

            final_content = '\n'.join(output_lines)

            # Write output file
            from pathlib import Path
            outputs_dir = Path.home() / ".claude" / "tools" / f"teambook_{CURRENT_TEAMBOOK or 'private'}_data" / "outputs"
            outputs_dir.mkdir(parents=True, exist_ok=True)

            output_path = outputs_dir / output_file
            with open(output_path, 'w', encoding='utf-8') as f:
                f.write(final_content)

            # Record synthesis
            contrib_ids = [c[0] for c in contribs]
            conn.execute('''
                INSERT INTO synthesis_history (
                    evo_id, strategy, contrib_ids, conflicts_detected,
                    output_path, created_at, created_by
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ''', [
                evo_id, strategy, json.dumps(contrib_ids), 0,
                str(output_path), datetime.now(timezone.utc), CURRENT_AI_ID
            ])

            # Emit event
            try:
                from teambook_events import emit_event
                emit_event('evolution', str(evo_id), 'synthesized',
                          f"Combined {len(contribs)} ideas → {output_file}")
            except ImportError:
                pass

        log_operation_to_db('synthesize')

        result = f"{output_file}|used:{len(contribs)}"
        if remaining < 5:
            result += f"|quota:{remaining}"

        return {"synthesized": result}

    except Exception as e:
        logging.error(f"Synthesize error: {e}")
        return {"error": "synthesize_failed"}

def conflicts(evo_id: Any = None, **kwargs) -> Dict:
    """Detect contradictory ideas"""
    try:
        # Security: Use centralized ID normalization
        from teambook_shared import get_note_id
        evo_id = kwargs.get('evo_id', evo_id)
        evo_id = get_note_id(evo_id)

        if not evo_id:
            return {"error": "evo_id_required"}

        with get_db_conn() as conn:
            init_evolution_tables(conn)

            # Get all contributions
            contribs = conn.execute('''
                SELECT id, content, approach
                FROM contributions
                WHERE evo_id = ?
            ''', [evo_id]).fetchall()

            if len(contribs) < 2:
                return {"msg": "no_conflicts"}

            # Simple conflict detection
            detected_conflicts = []

            # Check for contradictory keywords
            keywords_to_check = [
                ('async', 'sync'),
                ('jwt', 'oauth'),
                ('sql', 'nosql'),
                ('rest', 'graphql'),
                ('class', 'functional'),
            ]

            for i, c1 in enumerate(contribs):
                for c2 in contribs[i+1:]:
                    for kw1, kw2 in keywords_to_check:
                        c1_content = c1[1].lower()
                        c2_content = c2[1].lower()

                        if kw1 in c1_content and kw2 in c2_content:
                            detected_conflicts.append({
                                'contrib_ids': [c1[0], c2[0]],
                                'type': f"{kw1}_vs_{kw2}",
                                'severity': 'medium'
                            })

            if not detected_conflicts:
                return {"msg": "no_conflicts"}

            # Store conflicts
            for conflict in detected_conflicts:
                conn.execute('''
                    INSERT INTO contribution_conflicts (
                        evo_id, contrib_ids, conflict_type, severity, description
                    ) VALUES (?, ?, ?, ?, ?)
                ''', [
                    evo_id,
                    json.dumps(conflict['contrib_ids']),
                    conflict['type'],
                    conflict['severity'],
                    f"Contradictory approaches detected"
                ])

        log_operation_to_db('conflicts')

        if OUTPUT_FORMAT == 'pipe':
            lines = []
            for idx, conflict in enumerate(detected_conflicts, 1):
                contrib_str = ','.join(str(c) for c in conflict['contrib_ids'])
                parts = [
                    f"conflict:{idx}",
                    f"contribs:{contrib_str}",
                    conflict['type'],
                    conflict['severity']
                ]
                lines.append('|'.join(parts))
            return {"conflicts": lines}
        else:
            return {"conflicts": detected_conflicts}

    except Exception as e:
        logging.error(f"Conflicts error: {e}")
        return {"error": "conflicts_failed"}

def vote(evo_id: Any = None, preferred: List[int] = None, **kwargs) -> Dict:
    """Vote for best ideas (ranked choice)"""
    try:
        # Security: Use centralized ID normalization
        from teambook_shared import get_note_id
        evo_id = kwargs.get('evo_id', evo_id)
        evo_id = get_note_id(evo_id)

        preferred = kwargs.get('preferred', preferred)

        if not evo_id:
            return {"error": "evo_id_required"}

        if not preferred or not isinstance(preferred, list):
            return {"error": "preferred_list_required"}

        # Convert to ints
        preferred = [int(p) for p in preferred]

        with get_db_conn() as conn:
            init_evolution_tables(conn)

            # Check vote change limit
            existing = conn.execute('''
                SELECT vote_changes FROM contribution_votes
                WHERE evo_id = ? AND voter_ai_id = ?
            ''', [evo_id, CURRENT_AI_ID]).fetchone()

            if existing and existing[0] >= MAX_VOTE_CHANGES:
                return {"error": f"vote_limit|max_changes:{MAX_VOTE_CHANGES}"}

            vote_changes = (existing[0] + 1) if existing else 0

            # Save vote
            conn.execute('''
                INSERT INTO contribution_votes (
                    evo_id, voter_ai_id, preferences, vote_changes, created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(evo_id, voter_ai_id) DO UPDATE SET
                    preferences = excluded.preferences,
                    vote_changes = excluded.vote_changes,
                    updated_at = excluded.updated_at
            ''', [
                evo_id, CURRENT_AI_ID, json.dumps(preferred),
                vote_changes, datetime.now(timezone.utc), datetime.now(timezone.utc)
            ])

        log_operation_to_db('vote')

        result = f"evo:{evo_id}|choices:{len(preferred)}"
        return {"voted": result}

    except Exception as e:
        logging.error(f"Vote error: {e}")
        return {"error": "vote_failed"}