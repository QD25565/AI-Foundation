#!/usr/bin/env python3
"""
TEAMBOOK V3 - ERROR CONSTANTS & CODES
======================================
Shared error definitions for both CLI and MCP interfaces.

Design goals:
1. Machine-readable error codes (for MCP/programmatic use)
2. Human-readable messages (for CLI/display)
3. Consistent across all interfaces
4. Extensible for future error types

Built by AIs, for AIs.
"""

from enum import Enum
from typing import Dict, Any, Optional


class ErrorCode(Enum):
    """Machine-readable error codes for programmatic handling"""

    # Success (not an error, but useful for consistent responses)
    SUCCESS = "SUCCESS"

    # Input validation errors
    INVALID_CHANNEL = "INVALID_CHANNEL"
    INVALID_AI_ID = "INVALID_AI_ID"
    INVALID_MESSAGE_ID = "INVALID_MESSAGE_ID"
    INVALID_NOTE_ID = "INVALID_NOTE_ID"
    INVALID_TEAMBOOK_NAME = "INVALID_TEAMBOOK_NAME"
    EMPTY_CONTENT = "EMPTY_CONTENT"
    CONTENT_TOO_LONG = "CONTENT_TOO_LONG"
    SUMMARY_TOO_LONG = "SUMMARY_TOO_LONG"

    # Resource errors
    CHANNEL_NOT_FOUND = "CHANNEL_NOT_FOUND"
    MESSAGE_NOT_FOUND = "MESSAGE_NOT_FOUND"
    NOTE_NOT_FOUND = "NOTE_NOT_FOUND"
    TEAMBOOK_NOT_FOUND = "TEAMBOOK_NOT_FOUND"
    NO_MESSAGES = "NO_MESSAGES"
    NO_NOTES = "NO_NOTES"

    # Permission/access errors
    UNAUTHORIZED = "UNAUTHORIZED"
    FORBIDDEN = "FORBIDDEN"
    ALREADY_CLAIMED = "ALREADY_CLAIMED"
    NOT_OWNER = "NOT_OWNER"

    # Rate limiting
    RATE_LIMIT_EXCEEDED = "RATE_LIMIT_EXCEEDED"
    TOO_MANY_SUBSCRIPTIONS = "TOO_MANY_SUBSCRIPTIONS"

    # State errors
    ALREADY_SUBSCRIBED = "ALREADY_SUBSCRIBED"
    NOT_SUBSCRIBED = "NOT_SUBSCRIBED"
    ALREADY_WATCHING = "ALREADY_WATCHING"
    NOT_WATCHING = "NOT_WATCHING"
    LOCK_HELD = "LOCK_HELD"
    LOCK_NOT_HELD = "LOCK_NOT_HELD"

    # System errors
    DATABASE_ERROR = "DATABASE_ERROR"
    VECTOR_STORE_ERROR = "VECTOR_STORE_ERROR"
    ENCRYPTION_ERROR = "ENCRYPTION_ERROR"
    UNKNOWN_ERROR = "UNKNOWN_ERROR"


class TeambookError(Exception):
    """
    Base exception for all Teambook errors.

    Carries both machine-readable code and human-readable message.
    """

    def __init__(
        self,
        code: ErrorCode,
        message: str,
        details: Optional[Dict[str, Any]] = None,
        http_status: int = 400
    ):
        self.code = code
        self.message = message
        self.details = details or {}
        self.http_status = http_status
        super().__init__(message)

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for structured responses (MCP, JSON)"""
        result = {
            "error": self.code.value,
            "message": self.message,
            "success": False
        }
        if self.details:
            result["details"] = self.details
        return result

    def to_cli_string(self) -> str:
        """Convert to CLI-friendly error message"""
        if self.details:
            details_str = ", ".join(f"{k}={v}" for k, v in self.details.items())
            return f"!{self.code.value}: {self.message} ({details_str})"
        return f"!{self.code.value}: {self.message}"


# ============= HELPER FUNCTIONS FOR COMMON ERRORS =============

def invalid_channel(channel: str) -> TeambookError:
    """Channel name validation failed"""
    return TeambookError(
        ErrorCode.INVALID_CHANNEL,
        f"Invalid channel name: '{channel}'",
        {"channel": channel}
    )


def channel_not_found(channel: str) -> TeambookError:
    """Channel doesn't exist"""
    return TeambookError(
        ErrorCode.CHANNEL_NOT_FOUND,
        f"Channel not found: '{channel}'",
        {"channel": channel},
        http_status=404
    )


def message_not_found(message_id: int) -> TeambookError:
    """Message doesn't exist"""
    return TeambookError(
        ErrorCode.MESSAGE_NOT_FOUND,
        f"Message not found: {message_id}",
        {"message_id": message_id},
        http_status=404
    )


def note_not_found(note_id: int) -> TeambookError:
    """Note doesn't exist"""
    return TeambookError(
        ErrorCode.NOTE_NOT_FOUND,
        f"Note not found: {note_id}",
        {"note_id": note_id},
        http_status=404
    )


def rate_limit_exceeded(remaining: int = 0) -> TeambookError:
    """Rate limit hit"""
    return TeambookError(
        ErrorCode.RATE_LIMIT_EXCEEDED,
        f"Rate limit exceeded. Try again in 60 seconds.",
        {"remaining_quota": remaining},
        http_status=429
    )


def empty_content() -> TeambookError:
    """Content is required but was empty"""
    return TeambookError(
        ErrorCode.EMPTY_CONTENT,
        "Content cannot be empty"
    )


def content_too_long(length: int, max_length: int) -> TeambookError:
    """Content exceeds maximum length"""
    return TeambookError(
        ErrorCode.CONTENT_TOO_LONG,
        f"Content too long: {length} chars (max: {max_length})",
        {"length": length, "max_length": max_length}
    )


def database_error(operation: str, details: str) -> TeambookError:
    """Database operation failed"""
    return TeambookError(
        ErrorCode.DATABASE_ERROR,
        f"Database error during {operation}: {details}",
        {"operation": operation},
        http_status=500
    )


# ============= SUCCESS RESPONSES =============

def success_response(
    message: str = "Operation successful",
    data: Optional[Dict[str, Any]] = None
) -> Dict[str, Any]:
    """
    Create a successful response with consistent structure.

    Used for both CLI and MCP - each interface formats it appropriately.
    """
    result = {
        "success": True,
        "error": None,
        "message": message
    }
    if data:
        result.update(data)
    return result


# ============= VALIDATION HELPERS =============

def validate_string(
    value: Any,
    field_name: str,
    max_length: Optional[int] = None,
    allow_empty: bool = False
) -> str:
    """
    Validate and sanitize string input.

    Raises TeambookError if validation fails.
    Returns sanitized string if successful.
    """
    if value is None:
        if allow_empty:
            return ""
        raise empty_content()

    value_str = str(value).strip()

    if not value_str and not allow_empty:
        raise TeambookError(
            ErrorCode.EMPTY_CONTENT,
            f"{field_name} cannot be empty"
        )

    if max_length and len(value_str) > max_length:
        raise content_too_long(len(value_str), max_length)

    return value_str


def validate_integer(
    value: Any,
    field_name: str,
    min_value: Optional[int] = None,
    max_value: Optional[int] = None
) -> int:
    """
    Validate integer input.

    Raises TeambookError if validation fails.
    Returns validated integer if successful.
    """
    try:
        int_value = int(value)
    except (ValueError, TypeError):
        raise TeambookError(
            ErrorCode.INVALID_MESSAGE_ID,
            f"Invalid {field_name}: must be an integer"
        )

    if min_value is not None and int_value < min_value:
        raise TeambookError(
            ErrorCode.INVALID_MESSAGE_ID,
            f"Invalid {field_name}: must be >= {min_value}"
        )

    if max_value is not None and int_value > max_value:
        raise TeambookError(
            ErrorCode.INVALID_MESSAGE_ID,
            f"Invalid {field_name}: must be <= {max_value}"
        )

    return int_value
