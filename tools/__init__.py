"""
MCP Tools - Source Package
===========================
Auto-initializes awareness listener for real-time team coordination.
"""

import logging
import time

# Set up logging for initialization
log = logging.getLogger(__name__)

# Initialize awareness listener (auto-starts on import if Redis available)
# Also expose listener functions at package level
try:
    # Use relative import since we're in the tools package
    from .awareness_listener import (
        start_listener as _start_listener,
        stop_listener,
        get_listener_status,
        get_new_awareness_info
    )

    # Make them available at module level
    start_listener = _start_listener

    # Start the listener (thread-safe, can be called multiple times)
    _start_listener()

    # Give it a moment to initialize
    time.sleep(0.5)

    # Check if it started successfully
    status = get_listener_status()
    if status.get('running'):
        log.info("RedisAwarenessListener started successfully")
    elif not status.get('redis_available'):
        log.debug("Redis not available, awareness listener disabled")
    elif not status.get('enabled'):
        log.debug("Awareness listener disabled via config")
    else:
        log.warning("Awareness listener failed to start (check logs)")

except ImportError as e:
    log.debug(f"Awareness listener not available: {e}")
except Exception as e:
    log.warning(f"Failed to initialize awareness listener: {e}")