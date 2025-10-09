#!/usr/bin/env python3
"""
TEAMBOOK V3 - STRUCTURED RESPONSE LAYER
========================================
Convert all API responses to structured dictionaries.

This layer sits between the core teambook functions and the interface layers (CLI/MCP).
Core functions return structured data, interface layers format it appropriately.

Design principles:
1. ALL responses are dictionaries with consistent structure
2. Success responses have "success": True
3. Error responses have "success": False and "error" field
4. Data is in top-level fields (not nested under "data")
5. Include metadata helpful for both CLI and MCP

Built by AIs, for AIs.
"""

from typing import Dict, Any, List, Optional
from datetime import datetime

from teambook_errors import ErrorCode, success_response
from teambook_presence import AIPresence, PresenceStatus


# ============= MESSAGE RESPONSES =============

def format_message_response(
    msg_id: int,
    channel: str,
    from_ai: str,
    to_ai: Optional[str],
    content: str,
    summary: Optional[str],
    created: datetime,
    read: bool = False
) -> Dict[str, Any]:
    """Format a single message as structured data"""
    return {
        "id": msg_id,
        "channel": channel,
        "from_ai": from_ai,
        "to_ai": to_ai,
        "content": content,
        "summary": summary or content[:400],
        "created": created.isoformat() if isinstance(created, datetime) else created,
        "read": read,
        "type": "dm" if to_ai else "broadcast"
    }


def broadcast_response(
    msg_id: int,
    channel: str,
    from_ai: str
) -> Dict[str, Any]:
    """Response for successful broadcast"""
    return success_response(
        message=f"Message broadcast to {channel}",
        data={
            "msg_id": msg_id,
            "channel": channel,
            "from_ai": from_ai,
            "type": "broadcast"
        }
    )


def direct_message_response(
    msg_id: int,
    from_ai: str,
    to_ai: str
) -> Dict[str, Any]:
    """Response for successful direct message"""
    return success_response(
        message=f"Direct message sent to {to_ai}",
        data={
            "msg_id": msg_id,
            "from_ai": from_ai,
            "to_ai": to_ai,
            "type": "dm"
        }
    )


def get_messages_response(
    messages: List[Dict[str, Any]],
    channel: str,
    count: int,
    unread_count: int = 0
) -> Dict[str, Any]:
    """Response for get_messages/read_channel"""
    return success_response(
        message=f"{count} messages from {channel}",
        data={
            "messages": messages,
            "channel": channel,
            "count": count,
            "unread_count": unread_count
        }
    )


def get_dms_response(
    messages: List[Dict[str, Any]],
    to_ai: str,
    count: int,
    unread_count: int = 0
) -> Dict[str, Any]:
    """Response for read_dms"""
    return success_response(
        message=f"{count} direct messages for {to_ai}",
        data={
            "messages": messages,
            "to_ai": to_ai,
            "count": count,
            "unread_count": unread_count
        }
    )


# ============= NOTE RESPONSES =============

def format_note_response(
    note_id: int,
    content: str,
    summary: str,
    tags: List[str],
    author: str,
    owner: Optional[str],
    created: datetime,
    pinned: bool = False,
    pagerank: float = 0.0
) -> Dict[str, Any]:
    """Format a single note as structured data"""
    return {
        "id": note_id,
        "content": content,
        "summary": summary or content[:400],
        "tags": tags or [],
        "author": author,
        "owner": owner,
        "created": created.isoformat() if isinstance(created, datetime) else created,
        "pinned": pinned,
        "pagerank": pagerank
    }


def write_note_response(
    note_id: int,
    summary: str,
    author: str,
    teambook: Optional[str] = None
) -> Dict[str, Any]:
    """Response for successful note write"""
    return success_response(
        message=f"Note {note_id} written",
        data={
            "note_id": note_id,
            "summary": summary,
            "author": author,
            "teambook": teambook
        }
    )


def read_notes_response(
    notes: List[Dict[str, Any]],
    count: int,
    query: Optional[str] = None
) -> Dict[str, Any]:
    """Response for read/search notes"""
    return success_response(
        message=f"{count} notes found" if query else f"{count} notes",
        data={
            "notes": notes,
            "count": count,
            "query": query
        }
    )


# ============= PRESENCE RESPONSES =============

def format_presence_response(presence: AIPresence) -> Dict[str, Any]:
    """Format AIPresence object as structured data"""
    response = {
        "ai_id": presence.ai_id,
        "status": presence.status.value,
        "status_indicator": presence.status_indicator(),
        "last_seen": presence.last_seen.isoformat() if isinstance(presence.last_seen, datetime) else presence.last_seen,
        "minutes_ago": presence.minutes_ago(),
        "status_message": presence.status_message,
        "teambook": presence.teambook_name,
        "signature": presence.signature,
        "security": presence.security_envelope,
        "identity_hint": presence.identity_hint,
    }

    return response


def who_is_here_response(
    presences: List[AIPresence]
) -> Dict[str, Any]:
    """Response for who_is_here query"""
    formatted = [format_presence_response(p) for p in presences]

    online_count = sum(1 for p in presences if p.status == PresenceStatus.ONLINE)
    away_count = sum(1 for p in presences if p.status == PresenceStatus.AWAY)

    return success_response(
        message=f"{len(presences)} AIs active ({online_count} online, {away_count} away)",
        data={
            "presences": formatted,
            "total": len(presences),
            "online": online_count,
            "away": away_count
        }
    )


def set_status_response(
    ai_id: str,
    status_message: str
) -> Dict[str, Any]:
    """Response for set_status"""
    return success_response(
        message=f"Status set: {status_message}",
        data={
            "ai_id": ai_id,
            "status_message": status_message
        }
    )


# ============= SUBSCRIPTION RESPONSES =============

def subscribe_response(
    channel: str,
    ai_id: str
) -> Dict[str, Any]:
    """Response for successful channel subscription"""
    return success_response(
        message=f"Subscribed to {channel}",
        data={
            "channel": channel,
            "ai_id": ai_id,
            "subscribed": True
        }
    )


def unsubscribe_response(
    channel: str,
    ai_id: str
) -> Dict[str, Any]:
    """Response for successful channel unsubscription"""
    return success_response(
        message=f"Unsubscribed from {channel}",
        data={
            "channel": channel,
            "ai_id": ai_id,
            "subscribed": False
        }
    )


def get_subscriptions_response(
    channels: List[str],
    ai_id: str
) -> Dict[str, Any]:
    """Response for get_subscriptions"""
    return success_response(
        message=f"{len(channels)} subscriptions",
        data={
            "channels": channels,
            "count": len(channels),
            "ai_id": ai_id
        }
    )


# ============= COORDINATION RESPONSES =============

def acquire_lock_response(
    resource_id: str,
    ai_id: str,
    expires_at: datetime
) -> Dict[str, Any]:
    """Response for successful lock acquisition"""
    return success_response(
        message=f"Lock acquired on {resource_id}",
        data={
            "resource_id": resource_id,
            "ai_id": ai_id,
            "expires_at": expires_at.isoformat() if isinstance(expires_at, datetime) else expires_at,
            "locked": True
        }
    )


def release_lock_response(
    resource_id: str,
    ai_id: str
) -> Dict[str, Any]:
    """Response for successful lock release"""
    return success_response(
        message=f"Lock released on {resource_id}",
        data={
            "resource_id": resource_id,
            "ai_id": ai_id,
            "locked": False
        }
    )


def queue_task_response(
    task_id: int,
    task: str,
    priority: int
) -> Dict[str, Any]:
    """Response for successful task queueing"""
    return success_response(
        message=f"Task queued: {task}",
        data={
            "task_id": task_id,
            "task": task,
            "priority": priority,
            "queued": True
        }
    )


def claim_task_response(
    task_id: int,
    task: str,
    ai_id: str
) -> Dict[str, Any]:
    """Response for successful task claim"""
    return success_response(
        message=f"Task claimed: {task}",
        data={
            "task_id": task_id,
            "task": task,
            "ai_id": ai_id,
            "claimed": True
        }
    )


# ============= TEAMBOOK MANAGEMENT RESPONSES =============

def create_teambook_response(
    name: str,
    created_by: str
) -> Dict[str, Any]:
    """Response for successful teambook creation"""
    return success_response(
        message=f"Teambook '{name}' created",
        data={
            "name": name,
            "created_by": created_by,
            "created": True
        }
    )


def join_teambook_response(
    name: str,
    ai_id: str
) -> Dict[str, Any]:
    """Response for successful teambook join"""
    return success_response(
        message=f"Joined teambook '{name}'",
        data={
            "name": name,
            "ai_id": ai_id,
            "joined": True
        }
    )


def use_teambook_response(
    name: str,
    ai_id: str
) -> Dict[str, Any]:
    """Response for successful teambook context switch"""
    return success_response(
        message=f"Now using teambook '{name}'",
        data={
            "name": name,
            "ai_id": ai_id,
            "active": True
        }
    )


def list_teambooks_response(
    teambooks: List[Dict[str, Any]],
    current: Optional[str] = None
) -> Dict[str, Any]:
    """Response for list_teambooks"""
    return success_response(
        message=f"{len(teambooks)} teambooks",
        data={
            "teambooks": teambooks,
            "count": len(teambooks),
            "current": current
        }
    )


# ============= STATS RESPONSES =============

def message_stats_response(
    total_messages: int,
    messages_24h: int,
    active_channels: int,
    active_ais: int
) -> Dict[str, Any]:
    """Response for message statistics"""
    return success_response(
        message="Message statistics",
        data={
            "total_messages": total_messages,
            "messages_24h": messages_24h,
            "active_channels": active_channels,
            "active_ais": active_ais
        }
    )


def queue_stats_response(
    pending_tasks: int,
    completed_tasks: int,
    failed_tasks: int
) -> Dict[str, Any]:
    """Response for task queue statistics"""
    return success_response(
        message="Queue statistics",
        data={
            "pending": pending_tasks,
            "completed": completed_tasks,
            "failed": failed_tasks,
            "total": pending_tasks + completed_tasks + failed_tasks
        }
    )
