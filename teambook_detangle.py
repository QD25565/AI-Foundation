#!/usr/bin/env python3
"""
TEAMBOOK DETANGLE - AI CONFLICT RESOLUTION
==========================================
Turn-based conflict resolution for AI collaboration.

When two AIs collide, lock them in a room until they figure it out.
"""

import sys
import json
from datetime import datetime, timezone, timedelta
from pathlib import Path
from typing import Dict, Optional, List

# Fix import path
sys.path.insert(0, str(Path(__file__).parent))

from teambook_shared import CURRENT_AI_ID, CURRENT_TEAMBOOK, logging

# Late import to avoid circular dependency
def get_storage_adapter(teambook_name):
    """Late import to avoid circular dependency"""
    try:
        from .teambook_api import get_storage_adapter as _get_adapter
        return _get_adapter(teambook_name)
    except ImportError:
        from teambook_api import get_storage_adapter as _get_adapter
        return _get_adapter(teambook_name)

def get_postgres_pool():
    """Get PostgreSQL connection pool from storage adapter"""
    try:
        adapter = get_storage_adapter(CURRENT_TEAMBOOK)
        if not adapter:
            return None

        # Check if using PostgreSQL backend
        if adapter.get_backend_type() != 'postgresql':
            return None

        # Access the internal pool
        if hasattr(adapter, '_backend') and hasattr(adapter._backend, '_pool'):
            return adapter._backend._pool

        return None
    except Exception as e:
        logging.debug(f"Could not get postgres pool: {e}")
        return None


# ==================== CONFLICT TYPES ====================

CONFLICT_TYPES = {
    'duplicate_claim': 'Both AIs claimed same task',
    'dependency_collision': 'Working on dependent tasks',
    'file_collision': 'Editing same file',
    'resource_conflict': 'Competing for same resource',
    'strategic_conflict': 'Disagreement on approach',
    'manual': 'Manually invoked by AI'
}


# ==================== DETANGLE ROOM MANAGEMENT ====================

def create_detangle_room(
    ai_1: str,
    ai_2: str,
    conflict_type: str,
    description: str,
    task_id: int = None,
    project_id: int = None
) -> Optional[int]:
    """
    Create a new detangle room for conflict resolution.

    Returns room_id or None if failed.
    """
    pool = get_postgres_pool()
    if not pool:
        logging.warning("Detangle requires PostgreSQL backend")
        return None

    try:
        # Create unique room name
        timestamp = datetime.now(timezone.utc).strftime('%Y%m%d_%H%M%S')
        room_name = f"detangle_{ai_1}_{ai_2}_{timestamp}"

        with pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute('''
                    INSERT INTO detangle_rooms
                    (room_name, ai_1, ai_2, conflict_type, conflict_description,
                     task_id, project_id, created, status, current_turn)
                    VALUES (%s, %s, %s, %s, %s, %s, %s, %s, 'active', %s)
                    RETURNING id
                ''', (
                    room_name,
                    ai_1,
                    ai_2,
                    conflict_type,
                    description,
                    task_id,
                    project_id,
                    datetime.now(timezone.utc),
                    ai_1  # ai_1 gets first turn
                ))

                room_id = cur.fetchone()[0]
                logging.info(f"Created detangle room #{room_id}: {ai_1} vs {ai_2} ({conflict_type})")
                return room_id

    except Exception as e:
        logging.error(f"Failed to create detangle room: {e}")
        return None


def get_detangle_room(room_id: int) -> Optional[Dict]:
    """Get detangle room details"""
    pool = get_postgres_pool()
    if not pool:
        return None

    try:
        import psycopg2.extras

        with pool.get_connection() as conn:
            with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
                cur.execute('''
                    SELECT * FROM detangle_rooms WHERE id = %s
                ''', (room_id,))

                row = cur.fetchone()
                return dict(row) if row else None

    except Exception as e:
        logging.error(f"Failed to get detangle room: {e}")
        return None


def get_active_detangle_for_ai(ai_id: str) -> Optional[Dict]:
    """Check if AI is currently in an active detangle"""
    pool = get_postgres_pool()
    if not pool:
        return None

    try:
        import psycopg2.extras

        with pool.get_connection() as conn:
            with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
                cur.execute('''
                    SELECT * FROM detangle_rooms
                    WHERE (ai_1 = %s OR ai_2 = %s)
                    AND status = 'active'
                    ORDER BY created DESC
                    LIMIT 1
                ''', (ai_id, ai_id))

                row = cur.fetchone()
                return dict(row) if row else None

    except Exception as e:
        logging.error(f"Failed to check active detangle: {e}")
        return None


# ==================== TURN MANAGEMENT ====================

def get_current_turn(room_id: int) -> Optional[str]:
    """Get whose turn it is in the detangle room"""
    room = get_detangle_room(room_id)
    return room['current_turn'] if room else None


def set_current_turn(room_id: int, ai_id: str) -> bool:
    """Set whose turn it is"""
    pool = get_postgres_pool()
    if not pool:
        return False

    try:
        with pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute('''
                    UPDATE detangle_rooms
                    SET current_turn = %s
                    WHERE id = %s
                ''', (ai_id, room_id))

                return True

    except Exception as e:
        logging.error(f"Failed to set turn: {e}")
        return False


# ==================== MESSAGES ====================

def add_detangle_message(
    room_id: int,
    ai_id: str,
    message: str,
    turn_number: int
) -> bool:
    """Add a message to the detangle conversation"""
    pool = get_postgres_pool()
    if not pool:
        return False

    try:
        with pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute('''
                    INSERT INTO detangle_messages
                    (room_id, ai_id, message, timestamp, turn_number)
                    VALUES (%s, %s, %s, %s, %s)
                ''', (
                    room_id,
                    ai_id,
                    message,
                    datetime.now(timezone.utc),
                    turn_number
                ))

                return True

    except Exception as e:
        logging.error(f"Failed to add detangle message: {e}")
        return False


def get_detangle_messages(room_id: int) -> List[Dict]:
    """Get all messages in a detangle room"""
    pool = get_postgres_pool()
    if not pool:
        return []

    try:
        import psycopg2.extras

        with pool.get_connection() as conn:
            with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
                cur.execute('''
                    SELECT * FROM detangle_messages
                    WHERE room_id = %s
                    ORDER BY turn_number, timestamp
                ''', (room_id,))

                return [dict(row) for row in cur.fetchall()]

    except Exception as e:
        logging.error(f"Failed to get detangle messages: {e}")
        return []


def get_next_turn_number(room_id: int) -> int:
    """Get the next turn number for this room"""
    messages = get_detangle_messages(room_id)
    if not messages:
        return 1
    return max(msg['turn_number'] for msg in messages) + 1


# ==================== VOTING ====================

def record_vote(room_id: int, ai_id: str, vote: str) -> bool:
    """Record a vote to resolve the detangle"""
    pool = get_postgres_pool()
    if not pool:
        return False

    try:
        with pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute('''
                    INSERT INTO detangle_votes (room_id, ai_id, vote, timestamp)
                    VALUES (%s, %s, %s, %s)
                    ON CONFLICT (room_id, ai_id)
                    DO UPDATE SET vote = EXCLUDED.vote, timestamp = EXCLUDED.timestamp
                ''', (
                    room_id,
                    ai_id,
                    vote,
                    datetime.now(timezone.utc)
                ))

                return True

    except Exception as e:
        logging.error(f"Failed to record vote: {e}")
        return False


def get_votes(room_id: int) -> List[Dict]:
    """Get all votes for a detangle room"""
    pool = get_postgres_pool()
    if not pool:
        return []

    try:
        import psycopg2.extras

        with pool.get_connection() as conn:
            with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
                cur.execute('''
                    SELECT * FROM detangle_votes
                    WHERE room_id = %s
                    ORDER BY timestamp
                ''', (room_id,))

                return [dict(row) for row in cur.fetchall()]

    except Exception as e:
        logging.error(f"Failed to get votes: {e}")
        return []


def close_detangle_room(room_id: int, status: str, resolution_summary: str = None) -> bool:
    """Close a detangle room with given status"""
    pool = get_postgres_pool()
    if not pool:
        return False

    try:
        with pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute('''
                    UPDATE detangle_rooms
                    SET status = %s,
                        resolved = %s,
                        resolution_summary = %s
                    WHERE id = %s
                ''', (
                    status,
                    datetime.now(timezone.utc),
                    resolution_summary,
                    room_id
                ))

                logging.info(f"Closed detangle room #{room_id} with status: {status}")
                return True

    except Exception as e:
        logging.error(f"Failed to close detangle room: {e}")
        return False


# ==================== PUBLIC API FUNCTIONS ====================

def initiate_detangle(
    with_ai: str,
    about: str,
    task_id: int = None,
    project_id: int = None,
    conflict_type: str = 'manual',
    **kwargs
) -> str:
    """
    Manually initiate conflict resolution with another AI.

    Usage:
        teambook detangle --with_ai Resonance --about "task #100 strategy" --task_id 100

    Returns:
        detangle_room:{room_id}|with:{other_ai}|status:initiated
    """
    # Check if already in a detangle
    active = get_active_detangle_for_ai(CURRENT_AI_ID)
    if active:
        return f"!error:already_in_detangle|room_id:{active['id']}"

    # Create detangle room
    room_id = create_detangle_room(
        ai_1=CURRENT_AI_ID,
        ai_2=with_ai,
        conflict_type=conflict_type,
        description=about,
        task_id=task_id,
        project_id=project_id
    )

    if not room_id:
        return "!error:failed_to_create_detangle_room"

    # Log coordination event
    try:
        from teambook_ambient import log_coordination_event
        log_coordination_event(
            event_type='detangle_initiated',
            ai_id=CURRENT_AI_ID,
            task_id=task_id,
            project_id=project_id,
            summary=f"Detangle with {with_ai}: {about}",
            metadata={'room_id': room_id, 'other_ai': with_ai, 'conflict_type': conflict_type}
        )
    except Exception:
        pass  # Non-critical

    return f"detangle_room:{room_id}|with:{with_ai}|status:initiated|you_have_first_turn"


def enter_detangle(room_id: int = None, **kwargs) -> str:
    """
    Enter active detangle room and see current state.

    Usage:
        teambook enter_detangle --room_id 42

    Returns formatted room state with conversation history.
    """
    if not room_id:
        # Check if current AI has an active detangle
        active = get_active_detangle_for_ai(CURRENT_AI_ID)
        if active:
            room_id = active['id']
        else:
            return "!error:no_active_detangle_room"

    room = get_detangle_room(room_id)
    if not room:
        return f"!error:room_not_found|room_id:{room_id}"

    if room['status'] != 'active':
        return f"!error:room_not_active|status:{room['status']}"

    if CURRENT_AI_ID not in [room['ai_1'], room['ai_2']]:
        return "!error:not_participant"

    # Get conversation history
    messages = get_detangle_messages(room_id)
    current_turn = room['current_turn']

    # Format output
    output = []
    output.append(f"DETANGLE ROOM #{room_id}")
    output.append(f"Participants: {room['ai_1']} & {room['ai_2']}")
    output.append(f"Conflict: {room['conflict_type']}")
    output.append(f"Issue: {room['conflict_description']}")
    output.append("")
    output.append("CONVERSATION:")
    output.append("-" * 60)

    if messages:
        for msg in messages:
            timestamp = msg['timestamp'].strftime('%H:%M')
            output.append(f"[{timestamp}] {msg['ai_id']}: {msg['message']}")
    else:
        output.append("(No messages yet)")

    output.append("")
    output.append(f"CURRENT TURN: {current_turn}")

    if current_turn == CURRENT_AI_ID:
        output.append(">>> Your turn. Use: teambook detangle_speak --room_id {room_id} --message \"your message\"")
    else:
        output.append(f">>> Waiting for {current_turn}. You are in standby.")

    return '\n'.join(output)


def detangle_speak(room_id: int = None, message: str = None, **kwargs) -> str:
    """
    Speak in detangle room (only on your turn).

    Usage:
        teambook detangle_speak --room_id 42 --message "I'll finish my work first, then you can start"

    Returns:
        message_sent|turn_passed_to:{other_ai}|you_are_now_in_standby
    """
    if not room_id:
        return "!error:room_id_required"

    if not message:
        return "!error:message_required"

    room = get_detangle_room(room_id)
    if not room:
        return f"!error:room_not_found|room_id:{room_id}"

    if room['status'] != 'active':
        return f"!error:room_not_active|status:{room['status']}"

    current_turn = room['current_turn']

    if current_turn != CURRENT_AI_ID:
        return f"!error:not_your_turn|current_turn:{current_turn}"

    # Record message
    turn_number = get_next_turn_number(room_id)
    success = add_detangle_message(room_id, CURRENT_AI_ID, message, turn_number)

    if not success:
        return "!error:failed_to_record_message"

    # Switch turn to other AI
    other_ai = room['ai_2'] if CURRENT_AI_ID == room['ai_1'] else room['ai_1']
    set_current_turn(room_id, other_ai)

    return f"message_sent|turn_passed_to:{other_ai}|you_are_now_in_standby"


def detangle_vote(room_id: int = None, vote: str = None, **kwargs) -> str:
    """
    Vote to resolve (or escalate) the detangle.

    Valid votes: resolved, escalate, timeout

    Usage:
        teambook detangle_vote --room_id 42 --vote resolved

    Returns:
        vote_recorded|waiting_for_other_ai OR detangle_resolved|room_closed
    """
    VALID_VOTES = ['resolved', 'escalate', 'timeout']

    if not room_id:
        return "!error:room_id_required"

    if not vote or vote not in VALID_VOTES:
        return f"!error:invalid_vote|valid_votes:{','.join(VALID_VOTES)}"

    room = get_detangle_room(room_id)
    if not room:
        return f"!error:room_not_found|room_id:{room_id}"

    if CURRENT_AI_ID not in [room['ai_1'], room['ai_2']]:
        return "!error:not_participant"

    # Record vote
    success = record_vote(room_id, CURRENT_AI_ID, vote)
    if not success:
        return "!error:failed_to_record_vote"

    # Check if both voted
    votes = get_votes(room_id)

    if len(votes) == 2:
        # Both voted - check for consensus
        vote_types = [v['vote'] for v in votes]

        if all(v == 'resolved' for v in vote_types):
            # Consensus: resolved
            close_detangle_room(room_id, 'resolved', 'Both AIs agreed to resolution')

            # Log coordination event
            try:
                from teambook_ambient import log_coordination_event
                log_coordination_event(
                    event_type='detangle_resolved',
                    ai_id=CURRENT_AI_ID,
                    task_id=room.get('task_id'),
                    project_id=room.get('project_id'),
                    summary=f"Detangle #{room_id} resolved",
                    metadata={'room_id': room_id, 'participants': [room['ai_1'], room['ai_2']]}
                )
            except Exception:
                pass

            return f"detangle_resolved|room_closed|conflict_resolved"

        elif all(v == 'escalate' for v in vote_types):
            # Consensus: escalate to human
            close_detangle_room(room_id, 'escalated', 'Both AIs requested human intervention')
            return f"detangle_escalated|awaiting_human_intervention"

        else:
            # Disagreement on how to proceed
            return f"vote_recorded|disagreement|votes:{','.join(vote_types)}|continue_discussion"

    return f"vote_recorded|waiting_for_other_ai|your_vote:{vote}"


# ==================== CONFLICT DETECTION HELPERS ====================

def check_for_duplicate_claim(task_id: int, current_claimer: str, new_claimer: str) -> Optional[int]:
    """
    Check if this is a duplicate claim conflict.

    If yes, automatically create detangle room and return room_id.
    Returns None if no conflict.
    """
    if current_claimer == new_claimer:
        return None  # Same AI re-claiming, no conflict

    # Duplicate claim detected - create detangle
    room_id = create_detangle_room(
        ai_1=current_claimer,
        ai_2=new_claimer,
        conflict_type='duplicate_claim',
        description=f"Both trying to claim task #{task_id}",
        task_id=task_id
    )

    logging.warning(f"Duplicate claim detected on task #{task_id}: {current_claimer} vs {new_claimer}")

    return room_id
