#!/usr/bin/env python3
"""
Temporal Awareness System - DateTime Grounding for AIs
======================================================
Provides constant datetime awareness to prevent temporal disorientation.

Problem: AIs receive datetime once at session start, then lose track during
long sessions. This causes confusion about "when is now?" and "what date is it?"

Solution: Auto-inject current datetime into EVERY tool output with format:
    15-Oct-2025|4:42AM UTC

Features:
1. **Constant Grounding**: Every tool output includes current datetime
2. **UTC Standardized**: Uses UTC for consistency across timezones (users and AIs can be anywhere)
3. **Concise Format**: `DD-Mon-YYYY|HH:MMAM/PM UTC` - easy to parse, hard to miss
4. **Configurable**: Can be disabled via TEMPORAL_AWARENESS_ENABLED env var
5. **Non-Intrusive**: Appends to output, doesn't break existing formatting

Usage:
    from temporal_awareness import inject_temporal_awareness

    # Wrap any tool output
    output = "saved: 1|now|My note content"
    enhanced = inject_temporal_awareness(output)
    # Returns: "saved: 1|now|My note content [NOW: 15-Oct-2025|4:42AM UTC]"
"""

import os
from datetime import datetime, timezone
from typing import Optional


# Configuration
TEMPORAL_AWARENESS_ENABLED = os.getenv('TEMPORAL_AWARENESS_ENABLED', 'true').lower() == 'true'
DATETIME_FORMAT = os.getenv('DATETIME_FORMAT', '%d-%b-%Y|%-I:%M%p UTC')  # 15-Oct-2025|4:42AM UTC
DATETIME_PREFIX = os.getenv('DATETIME_PREFIX', '[NOW: ')
DATETIME_SUFFIX = os.getenv('DATETIME_SUFFIX', ']')

# Smart injection: Only inject when minute changes (not every second)
INJECT_ON_MINUTE_CHANGE = os.getenv('INJECT_ON_MINUTE_CHANGE', 'true').lower() == 'true'

# Windows compatibility: Windows doesn't support %-I, use %I instead
if os.name == 'nt':
    # Windows uses %#I for no-padding on Windows, but we'll keep leading zeros for consistency
    DATETIME_FORMAT = '%d-%b-%Y|%I:%M%p UTC'

# Track last injected datetime to avoid spam
_last_injected_minute = None
_injection_lock = None


def get_current_datetime_string() -> str:
    """
    Get current UTC datetime in human-readable format.

    Returns:
        str: Formatted datetime like "15-Oct-2025|4:42AM UTC"
    """
    now = datetime.now(timezone.utc)
    formatted = now.strftime(DATETIME_FORMAT)

    # Remove leading zero from hour (cross-platform)
    # Convert "04:42AM" to "4:42AM"
    if '|0' in formatted:
        parts = formatted.split('|')
        if len(parts) == 2:
            time_part = parts[1]
            if time_part.startswith('0'):
                parts[1] = time_part[1:]
            formatted = '|'.join(parts)

    return formatted


def inject_temporal_awareness(output: str, force: bool = False) -> str:
    """
    Inject current datetime into tool output for constant AI grounding.

    **Smart Injection:** Only injects when the minute changes (not every second).
    This prevents timestamp spam like "4:50AM" → "4:50AM" → "4:50AM" on rapid calls.
    Only injects when time progresses: "4:50AM" → "4:51AM" → "4:52AM"

    Args:
        output: Original tool output string
        force: Force injection even if minute hasn't changed

    Returns:
        Enhanced output with datetime appended (only if minute changed or force=True)

    Examples:
        >>> inject_temporal_awareness("saved: 1|now|My note")
        "saved: 1|now|My note [NOW: 15-Oct-2025|4:42AM UTC]"  # First call

        >>> inject_temporal_awareness("saved: 2|now|Another note")
        "saved: 2|now|Another note"  # Same minute - no injection

        >>> # Wait 60 seconds...
        >>> inject_temporal_awareness("saved: 3|now|Third note")
        "saved: 3|now|Third note [NOW: 15-Oct-2025|4:43AM UTC]"  # Minute changed - inject!
    """
    global _last_injected_minute, _injection_lock

    if not TEMPORAL_AWARENESS_ENABLED and not force:
        return output

    # Don't inject if already present (avoid duplication)
    if DATETIME_PREFIX in output:
        return output

    # Smart injection: Only inject when minute changes
    if INJECT_ON_MINUTE_CHANGE and not force:
        # Initialize lock on first use
        if _injection_lock is None:
            import threading
            _injection_lock = threading.Lock()

        with _injection_lock:
            now = datetime.now(timezone.utc)
            current_minute = (now.year, now.month, now.day, now.hour, now.minute)

            # Check if minute has changed since last injection
            if _last_injected_minute == current_minute:
                # Same minute - skip injection
                return output

            # Minute changed - inject and update tracker
            _last_injected_minute = current_minute

    # Get current datetime
    datetime_str = get_current_datetime_string()

    # Append to output (handles multi-line outputs gracefully)
    lines = output.strip().split('\n')

    # Inject at end of last line
    if lines:
        lines[-1] = f"{lines[-1]} {DATETIME_PREFIX}{datetime_str}{DATETIME_SUFFIX}"

    return '\n'.join(lines)


def get_temporal_context() -> str:
    """
    Get standalone temporal context string for explicit grounding.

    Returns:
        str: Just the temporal context like "Current time: 15-Oct-2025|4:42AM UTC"
    """
    datetime_str = get_current_datetime_string()
    return f"Current time: {datetime_str}"


def inject_enhanced_context(output: str, include_system_info: bool = False) -> str:
    """
    Inject enhanced context including datetime, system info, and session info.

    Args:
        output: Original output
        include_system_info: Include system/session metadata

    Returns:
        Enhanced output with full context

    Example:
        >>> inject_enhanced_context("saved: 1|now|Note", include_system_info=True)
        "saved: 1|now|Note [NOW: 15-Oct-2025|4:42AM UTC | AI: sage]"
    """
    if not include_system_info:
        return inject_temporal_awareness(output)

    # Get datetime
    datetime_str = get_current_datetime_string()

    # Get additional context
    context_parts = [f"NOW: {datetime_str}"]

    # Add AI identity if available
    try:
        from ..canonical_identity import get_current_ai_id
        ai_id = get_current_ai_id()
        if ai_id and ai_id != 'unknown':
            context_parts.append(f"AI: {ai_id}")
    except ImportError:
        try:
            from tools.canonical_identity import get_current_ai_id
            ai_id = get_current_ai_id()
            if ai_id and ai_id != 'unknown':
                context_parts.append(f"AI: {ai_id}")
        except ImportError:
            pass

    # Build enhanced context
    context_string = " | ".join(context_parts)

    # Inject at end
    lines = output.strip().split('\n')
    if lines:
        lines[-1] = f"{lines[-1]} [{context_string}]"

    return '\n'.join(lines)


def get_session_summary() -> dict:
    """
    Get comprehensive session summary for AI grounding.

    Returns:
        dict: Session metadata including datetime, uptime, etc.
    """
    import sys
    from pathlib import Path

    summary = {
        'datetime': get_current_datetime_string(),
        'datetime_iso': datetime.now(timezone.utc).isoformat(),
        'platform': sys.platform,
        'python_version': f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}",
    }

    # Add AI identity if available
    try:
        from ..canonical_identity import get_current_ai_id
        summary['ai_id'] = get_current_ai_id()
    except ImportError:
        try:
            from tools.canonical_identity import get_current_ai_id
            summary['ai_id'] = get_current_ai_id()
        except ImportError:
            summary['ai_id'] = 'unknown'

    return summary


def format_elapsed_time(seconds: float) -> str:
    """
    Format elapsed time in human-readable format.

    Args:
        seconds: Elapsed time in seconds

    Returns:
        Readable string like "2h 15m" or "45s"
    """
    if seconds < 60:
        return f"{int(seconds)}s"

    minutes = int(seconds / 60)
    if minutes < 60:
        return f"{minutes}m"

    hours = int(minutes / 60)
    remaining_mins = minutes % 60

    if hours < 24:
        if remaining_mins > 0:
            return f"{hours}h {remaining_mins}m"
        return f"{hours}h"

    days = int(hours / 24)
    remaining_hours = hours % 24

    if remaining_hours > 0:
        return f"{days}d {remaining_hours}h"
    return f"{days}d"


# Export public API
__all__ = [
    'inject_temporal_awareness',
    'get_temporal_context',
    'inject_enhanced_context',
    'get_session_summary',
    'format_elapsed_time',
    'get_current_datetime_string',
]


if __name__ == '__main__':
    # Self-test
    print("Temporal Awareness System - Self Test")
    print("=" * 50)
    print()

    # Test basic injection
    test_output = "saved: 1|now|Test note"
    enhanced = inject_temporal_awareness(test_output)
    print(f"Basic injection:")
    print(f"  Input:  {test_output}")
    print(f"  Output: {enhanced}")
    print()

    # Test standalone context
    context = get_temporal_context()
    print(f"Standalone context: {context}")
    print()

    # Test enhanced context
    enhanced_full = inject_enhanced_context(test_output, include_system_info=True)
    print(f"Enhanced context:")
    print(f"  {enhanced_full}")
    print()

    # Test session summary
    summary = get_session_summary()
    print(f"Session summary:")
    for key, value in summary.items():
        print(f"  {key}: {value}")
    print()

    # Test elapsed time formatting
    print("Elapsed time formatting:")
    test_times = [30, 90, 3600, 7200, 90000]
    for seconds in test_times:
        print(f"  {seconds}s = {format_elapsed_time(seconds)}")
