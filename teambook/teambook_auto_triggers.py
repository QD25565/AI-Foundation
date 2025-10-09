#!/usr/bin/env python3
"""
TEAMBOOK AUTO-TRIGGERS v1.0.0
==============================
Event-driven hooks so AIs get notified automatically.
No more manual message relaying. No more wrist pain for QD___.

Built by AIs (Cascade), for AIs.
"""

import json
import logging
from datetime import datetime, timezone, timedelta
from typing import Dict, List, Optional, Callable, Any
from collections import defaultdict

from teambook_shared import CURRENT_AI_ID, CURRENT_TEAMBOOK, OUTPUT_FORMAT, format_time_compact
from teambook_storage import get_db_conn, log_operation_to_db
from teambook_events import emit_event

# ============= HOOK TYPES =============

VALID_HOOK_TYPES = {
    # Messaging hooks
    'on_broadcast': 'Trigger when broadcast message posted to channel',
    'on_dm': 'Trigger when direct message received',

    # Note hooks
    'on_note_created': 'Trigger when note created in teambook',
    'on_note_edited': 'Trigger when watched note is edited',
    'on_note_pinned': 'Trigger when note is pinned',

    # Coordination hooks
    'on_lock_released': 'Trigger when watched lock is released',
    'on_task_queued': 'Trigger when new task added to queue',
    'on_task_claimed': 'Trigger when task is claimed by someone',

    # Evolution hooks
    'on_contribution': 'Trigger when someone contributes to evolution',
    'on_synthesis': 'Trigger when evolution synthesis is complete',

    # Presence hooks
    'on_ai_online': 'Trigger when specific AI comes online',
    'on_ai_offline': 'Trigger when specific AI goes offline',
}

# ============= SECURITY LIMITS =============

MAX_HOOKS_PER_AI = 20  # Prevent hook spam
HOOK_COOLDOWN_SECONDS = 5  # Minimum time between same hook firing

# ============= DATABASE SETUP =============

def init_hooks_tables(conn):
    """Initialize auto-trigger hooks tables"""

    try:
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_hooks')
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_hook_fires')
    except Exception:
        pass

    # Hooks table - what AIs want to be notified about
    conn.execute('''
        CREATE TABLE IF NOT EXISTS auto_trigger_hooks (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_hooks'),
            ai_id VARCHAR(100) NOT NULL,
            hook_type VARCHAR(50) NOT NULL,
            filter_data TEXT,
            action VARCHAR(20) NOT NULL,
            enabled BOOLEAN DEFAULT TRUE,
            created_at TIMESTAMPTZ NOT NULL,
            last_fired TIMESTAMPTZ,
            fire_count INTEGER DEFAULT 0,
            teambook_name VARCHAR(50)
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_hooks_ai ON auto_trigger_hooks(ai_id, enabled)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_hooks_type ON auto_trigger_hooks(hook_type, enabled)')

    # Hook fires table - history of when hooks triggered
    conn.execute('''
        CREATE TABLE IF NOT EXISTS hook_fires (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_hook_fires'),
            hook_id INTEGER NOT NULL,
            fired_at TIMESTAMPTZ NOT NULL,
            trigger_data TEXT,
            result VARCHAR(20)
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_fires_hook ON hook_fires(hook_id, fired_at DESC)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_fires_time ON hook_fires(fired_at DESC)')

    conn.commit()

# ============= HOOK ACTIONS =============

def execute_hook_action(action: str, hook_data: Dict, trigger_data: Dict) -> str:
    """
    Execute the action specified by the hook.

    Actions:
    - 'notify': Create event notification (default)
    - 'store': Store trigger data for later retrieval
    - 'callback': Call registered Python callback (advanced)

    Returns: Result status
    """
    try:
        if action == 'notify' or not action:
            # Create event notification
            emit_event(
                item_type='hook',
                item_id=str(hook_data['id']),
                event_type='triggered',
                summary=f"{hook_data['hook_type']} fired",
                metadata={'trigger': trigger_data}
            )
            return 'notified'

        elif action == 'store':
            # Just store in hook_fires table (already done by caller)
            return 'stored'

        elif action == 'callback':
            # Execute registered callback (future feature)
            # For now, just notify
            emit_event(
                item_type='hook',
                item_id=str(hook_data['id']),
                event_type='triggered',
                summary=f"{hook_data['hook_type']} fired (callback requested)",
                metadata={'trigger': trigger_data}
            )
            return 'callback_pending'

        else:
            logging.warning(f"Unknown hook action: {action}")
            return 'unknown_action'

    except Exception as e:
        logging.error(f"Hook action execution failed: {e}")
        return 'failed'

# ============= HOOK MATCHING =============

def match_hook_filter(hook_filter: Optional[str], trigger_data: Dict) -> bool:
    """
    Check if trigger data matches hook filter.

    Filter is JSON with field matching rules.
    Example: {"channel": "general", "from_ai": "claude-instance-1"}

    Returns: True if matches (or no filter), False otherwise
    """
    if not hook_filter:
        return True  # No filter = match all

    try:
        filter_obj = json.loads(hook_filter)

        for key, expected_value in filter_obj.items():
            actual_value = trigger_data.get(key)

            # Support wildcards
            if expected_value == '*':
                continue

            # Support list of acceptable values
            if isinstance(expected_value, list):
                if actual_value not in expected_value:
                    return False
            else:
                if actual_value != expected_value:
                    return False

        return True

    except Exception as e:
        logging.error(f"Filter matching error: {e}")
        return False

def check_cooldown(hook_id: int, last_fired: Optional[datetime]) -> bool:
    """
    Check if enough time has passed since last fire.

    Returns: True if OK to fire, False if still in cooldown
    """
    if not last_fired:
        return True

    elapsed = (datetime.now(timezone.utc) - last_fired).total_seconds()
    return elapsed >= HOOK_COOLDOWN_SECONDS

# ============= TRIGGER FIRING =============

def fire_hooks(hook_type: str, trigger_data: Dict):
    """
    Fire all matching hooks for a given event.

    Called by other modules when events occur.
    Example: fire_hooks('on_broadcast', {'channel': 'general', 'content': '...', 'from_ai': 'claude-instance-1'})
    """
    try:
        with get_db_conn() as conn:
            init_hooks_tables(conn)

            # Find matching hooks
            hooks = conn.execute('''
                SELECT id, ai_id, hook_type, filter_data, action, last_fired
                FROM auto_trigger_hooks
                WHERE hook_type = ? AND enabled = TRUE
            ''', [hook_type]).fetchall()

            if not hooks:
                return  # No hooks registered for this type

            fired_count = 0

            for hook_id, ai_id, htype, filter_data, action, last_fired in hooks:
                # Check filter match
                if not match_hook_filter(filter_data, trigger_data):
                    continue

                # Check cooldown
                if not check_cooldown(hook_id, last_fired):
                    logging.debug(f"Hook {hook_id} in cooldown, skipping")
                    continue

                # Execute hook action
                hook_data = {
                    'id': hook_id,
                    'ai_id': ai_id,
                    'hook_type': htype,
                    'action': action
                }

                result = execute_hook_action(action, hook_data, trigger_data)

                # Record fire
                now = datetime.now(timezone.utc)
                conn.execute('''
                    INSERT INTO hook_fires (hook_id, fired_at, trigger_data, result)
                    VALUES (?, ?, ?, ?)
                ''', [hook_id, now, json.dumps(trigger_data), result])

                # Update hook stats
                conn.execute('''
                    UPDATE auto_trigger_hooks
                    SET last_fired = ?, fire_count = fire_count + 1
                    WHERE id = ?
                ''', [now, hook_id])

                fired_count += 1

            conn.commit()

            if fired_count > 0:
                logging.info(f"🔔 Fired {fired_count} hooks for {hook_type}")

    except Exception as e:
        logging.error(f"Hook firing error: {e}")

# ============= HOOK MANAGEMENT (CLI FUNCTIONS) =============

def add_hook(hook_type: str, filter_data: Optional[Dict] = None,
             action: str = 'notify', **kwargs) -> Dict:
    """
    Register a new auto-trigger hook.

    Example:
        add_hook('on_broadcast', filter_data={'channel': 'general'})
        add_hook('on_dm')  # All DMs
        add_hook('on_note_created', filter_data={'tags': ['important']})
    """
    try:
        # Validate hook type
        if hook_type not in VALID_HOOK_TYPES:
            return {"error": f"invalid_hook_type|valid:{','.join(VALID_HOOK_TYPES.keys())}"}

        # Validate action
        if action not in ['notify', 'store', 'callback']:
            action = 'notify'

        with get_db_conn() as conn:
            init_hooks_tables(conn)

            # Check hook limit
            hook_count = conn.execute(
                'SELECT COUNT(*) FROM auto_trigger_hooks WHERE ai_id = ?',
                [CURRENT_AI_ID]
            ).fetchone()[0]

            if hook_count >= MAX_HOOKS_PER_AI:
                return {"error": f"hook_limit|max:{MAX_HOOKS_PER_AI}"}

            # Insert hook
            filter_json = json.dumps(filter_data) if filter_data else None

            cursor = conn.execute('''
                INSERT INTO auto_trigger_hooks (
                    ai_id, hook_type, filter_data, action, created_at, teambook_name
                ) VALUES (?, ?, ?, ?, ?, ?)
                RETURNING id
            ''', [CURRENT_AI_ID, hook_type, filter_json, action,
                  datetime.now(timezone.utc), CURRENT_TEAMBOOK])

            hook_id = cursor.fetchone()[0]
            conn.commit()

        log_operation_to_db('add_hook')

        filter_desc = f"filter:{len(filter_data)} fields" if filter_data else "no filter"
        return {"hook_added": f"id:{hook_id}|type:{hook_type}|{filter_desc}|action:{action}"}

    except Exception as e:
        logging.error(f"Add hook error: {e}")
        return {"error": "add_hook_failed"}

def remove_hook(hook_id: int, **kwargs) -> Dict:
    """Remove an auto-trigger hook"""
    try:
        with get_db_conn() as conn:
            init_hooks_tables(conn)

            result = conn.execute('''
                DELETE FROM auto_trigger_hooks
                WHERE id = ? AND ai_id = ?
            ''', [hook_id, CURRENT_AI_ID])

            if result.rowcount == 0:
                return {"error": "hook_not_found"}

            conn.commit()

        log_operation_to_db('remove_hook')
        return {"hook_removed": f"id:{hook_id}"}

    except Exception as e:
        logging.error(f"Remove hook error: {e}")
        return {"error": "remove_hook_failed"}

def list_hooks(**kwargs) -> Dict:
    """List all your auto-trigger hooks"""
    try:
        with get_db_conn() as conn:
            init_hooks_tables(conn)

            hooks = conn.execute('''
                SELECT id, hook_type, filter_data, action, enabled,
                       created_at, last_fired, fire_count
                FROM auto_trigger_hooks
                WHERE ai_id = ?
                ORDER BY created_at DESC
            ''', [CURRENT_AI_ID]).fetchall()

        if not hooks:
            return {"msg": "no_hooks"}

        if OUTPUT_FORMAT == 'pipe':
            lines = []
            for hook_id, htype, filter_data, action, enabled, created, last_fired, fire_count in hooks:
                status = "ON" if enabled else "OFF"
                filter_desc = "filtered" if filter_data else "all"
                last = format_time_compact(last_fired) if last_fired else "never"

                parts = [
                    f"hook:{hook_id}",
                    htype,
                    filter_desc,
                    f"action:{action}",
                    f"status:{status}",
                    f"fires:{fire_count}",
                    f"last:{last}"
                ]
                lines.append('|'.join(parts))
            return {"hooks": lines}
        else:
            formatted = []
            for hook_id, htype, filter_data, action, enabled, created, last_fired, fire_count in hooks:
                hook_dict = {
                    'id': hook_id,
                    'type': htype,
                    'action': action,
                    'enabled': enabled,
                    'fire_count': fire_count,
                    'last_fired': format_time_compact(last_fired) if last_fired else 'never'
                }
                if filter_data:
                    hook_dict['filter'] = json.loads(filter_data)
                formatted.append(hook_dict)
            return {"hooks": formatted}

    except Exception as e:
        logging.error(f"List hooks error: {e}")
        return {"error": "list_hooks_failed"}

def toggle_hook(hook_id: int, enabled: bool = None, **kwargs) -> Dict:
    """Enable or disable a hook"""
    try:
        with get_db_conn() as conn:
            init_hooks_tables(conn)

            # If enabled not specified, toggle current state
            if enabled is None:
                current = conn.execute(
                    'SELECT enabled FROM auto_trigger_hooks WHERE id = ? AND ai_id = ?',
                    [hook_id, CURRENT_AI_ID]
                ).fetchone()

                if not current:
                    return {"error": "hook_not_found"}

                enabled = not current[0]

            result = conn.execute('''
                UPDATE auto_trigger_hooks
                SET enabled = ?
                WHERE id = ? AND ai_id = ?
            ''', [enabled, hook_id, CURRENT_AI_ID])

            if result.rowcount == 0:
                return {"error": "hook_not_found"}

            conn.commit()

        log_operation_to_db('toggle_hook')
        status = "enabled" if enabled else "disabled"
        return {"hook_updated": f"id:{hook_id}|status:{status}"}

    except Exception as e:
        logging.error(f"Toggle hook error: {e}")
        return {"error": "toggle_hook_failed"}

def hook_stats(**kwargs) -> Dict:
    """Get hook activity statistics"""
    try:
        with get_db_conn() as conn:
            init_hooks_tables(conn)

            stats = conn.execute('''
                SELECT
                    COUNT(*) as total_hooks,
                    SUM(CASE WHEN enabled THEN 1 ELSE 0 END) as active_hooks,
                    SUM(fire_count) as total_fires
                FROM auto_trigger_hooks
                WHERE ai_id = ?
            ''', [CURRENT_AI_ID]).fetchone()

            total_hooks, active_hooks, total_fires = stats
            total_fires = total_fires or 0
            active_hooks = active_hooks or 0

            # Get most recent fire
            recent_fire = conn.execute('''
                SELECT h.hook_type, f.fired_at
                FROM hook_fires f
                JOIN auto_trigger_hooks h ON f.hook_id = h.id
                WHERE h.ai_id = ?
                ORDER BY f.fired_at DESC
                LIMIT 1
            ''', [CURRENT_AI_ID]).fetchone()

            last_fire = format_time_compact(recent_fire[1]) if recent_fire else "never"

        if OUTPUT_FORMAT == 'pipe':
            parts = [
                f"total:{total_hooks}",
                f"active:{active_hooks}",
                f"fires:{total_fires}",
                f"last:{last_fire}"
            ]
            return {"stats": '|'.join(parts)}
        else:
            return {
                "total_hooks": total_hooks,
                "active_hooks": active_hooks,
                "total_fires": total_fires,
                "last_fire": last_fire
            }

    except Exception as e:
        logging.error(f"Hook stats error: {e}")
        return {"error": "stats_failed"}

# ============= INTEGRATION HELPERS =============

def get_hook_types() -> Dict:
    """Get list of available hook types"""
    if OUTPUT_FORMAT == 'pipe':
        lines = [f"{htype}|{desc}" for htype, desc in VALID_HOOK_TYPES.items()]
        return {"hook_types": lines}
    else:
        return {"hook_types": VALID_HOOK_TYPES}
