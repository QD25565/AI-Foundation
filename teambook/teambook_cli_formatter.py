#!/usr/bin/env python3
"""
TEAMBOOK CLI FORMATTER
==========================
Formats structured dict responses from API into beautiful CLI output.

This is the presentation layer - takes structured data and makes it human-friendly.
"""

from typing import Dict, Any
from datetime import datetime


def format_send_message(result: Dict[str, Any]) -> str:
    """
    Format send_message result for CLI display.

    Input: {"success": True, "msg_id": 123, "from": "instance-1", ...}
    Output: Pretty formatted string with emojis and info
    """
    if not result.get("success"):
        return format_error(result)

    # Success case
    lines = []
    lines.append("Message sent successfully")

    # Message ID and routing
    msg_id = result.get("msg_id")
    from_ai = result.get("from")
    to = result.get("to")
    channel = result.get("channel")

    if to == "all":
        # Broadcast
        lines.append(f"msg:{msg_id}|{from_ai}->all|#{channel}|now")
    else:
        # Direct message
        lines.append(f"dm:{msg_id}|{from_ai}->{to}|now")

    # Reply indicator
    if result.get("reply_to"):
        lines.append(f"  Reply to msg:{result['reply_to']}")

    # Recipient count (for broadcasts)
    if to == "all":
        count = result.get("recipients_count", 0)
        lines.append(f"Delivered to {count} active AIs")

    # Warnings
    if result.get("warning"):
        warning_msg = result.get("warning_message", "")
        lines.append(f"Warning: {warning_msg}")

    # Quota warning
    if result.get("quota_remaining"):
        remaining = result["quota_remaining"]
        lines.append(f"Rate limit: {remaining} messages remaining")

    return '\n'.join(lines)


def format_get_messages(result: Dict[str, Any]) -> str:
    """
    Format get_messages result for CLI display.

    Input: {"success": True, "messages": [...], ...}
    Output: Pretty message list with attribution and status
    """
    if not result.get("success"):
        return format_error(result)

    lines = []

    # Header
    channel = result.get("channel", "unknown")
    count = result.get("count", 0)
    compact = result.get("compact", True)

    mode = "compact" if compact else "full"
    lines.append(f"Channel: #{channel} ({count} messages, {mode} mode)\n")

    # Messages
    messages = result.get("messages", [])
    if not messages:
        lines.append("  (no messages)")
        return '\n'.join(lines)

    for msg in messages:
        msg_id = msg.get("id")
        from_ai = msg.get("from")
        to = msg.get("to")
        summary = msg.get("summary", "")
        reply_to = msg.get("reply_to")
        unread = msg.get("unread", False)

        # Format: ID|FROM->TO|TIMESTAMP|SUMMARY|STATUS
        status = "unread" if unread else "read"
        reply_prefix = "  > " if reply_to else ""

        lines.append(f"{reply_prefix}{msg_id}|{from_ai}->{to}|{summary}|{status}")

        # Show full content if not compact
        if msg.get("content") and not compact:
            content = msg["content"]
            # Indent content
            for line in content.split('\n'):
                lines.append(f"    {line}")
            lines.append("")  # Blank line after full content

    # Footer
    if result.get("has_more"):
        lines.append(f"\nMore messages available. Use --since {messages[-1]['id']} to load next batch")

    return '\n'.join(lines)


def format_error(result: Dict[str, Any]) -> str:
    """
    Format error response for CLI display.

    Input: {"success": False, "error": "rate_limit", "message": "...", ...}
    Output: Pretty error with suggestions
    """
    error_code = result.get("error", "unknown_error")
    message = result.get("message", "An error occurred")
    details = result.get("details", {})
    suggestion = result.get("suggestion")

    lines = []
    lines.append(f"Error: {message}")

    # Details
    if details:
        for key, value in details.items():
            lines.append(f"   {key}: {value}")

    # Suggestion
    if suggestion:
        lines.append(f"Suggestion: {suggestion}")

    # Error code (for debugging)
    lines.append(f"   (error code: {error_code})")

    return '\n'.join(lines)


def format_dict_generic(result: Dict[str, Any], command_name: str = "command") -> str:
    """
    Generic formatter for any result dict.
    Falls back to simple key: value display.
    """
    if not isinstance(result, dict):
        return str(result)

    # If it's an error, use error formatter
    if not result.get("success"):
        return format_error(result)

    # Otherwise, simple key-value display
    lines = [f"{command_name} succeeded\n"]
    for key, value in result.items():
        if key == "success":
            continue
        if isinstance(value, (list, dict)):
            lines.append(f"{key}: {len(value) if isinstance(value, list) else '...'}")
        else:
            lines.append(f"{key}: {value}")

    return '\n'.join(lines)


# Formatter registry - maps function names to formatters
FORMATTERS = {
    "send_message": format_send_message,
    "get_messages": format_get_messages,
}


def format_result(result: Any, function_name: str = None) -> str:
    """
    Main entry point for formatting results.

    Args:
        result: The dict returned by a function
        function_name: Name of the function that produced this result

    Returns:
        Beautifully formatted string for CLI display
    """
    # If it's not a dict, just return as string
    if not isinstance(result, dict):
        return str(result)

    # Find the right formatter
    formatter = FORMATTERS.get(function_name, format_dict_generic)

    return formatter(result)
