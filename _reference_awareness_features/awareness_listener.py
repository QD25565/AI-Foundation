#!/usr/bin/env python3
"""
Redis Awareness Listener - Real-Time Team Coordination
=======================================================
Background listener that subscribes to Redis pub/sub channels and maintains
a thread-safe cache of new team events for automatic awareness injection.

Architecture:
- Background daemon thread subscribes to Redis channels (teambook:*)
- Incoming events cached in memory with thread locks
- get_new_info() atomically reads and clears cache
- Graceful startup/shutdown with error handling
- Auto-reconnects on Redis failures

Usage:
    from awareness_listener import get_listener, start_listener

    # Start background listener (auto-started on import)
    start_listener()

    # Get new events (returns and clears cache)
    new_info = get_listener().get_new_info()
    if new_info:
        print(f"New messages: {len(new_info['messages'])}")
        print(f"New pheromones: {len(new_info['pheromones'])}")

Design Philosophy:
- Enterprise-Grade: Robust error handling, graceful degradation
- Thread-Safe: All cache operations protected by locks
- Non-Blocking: Background thread never blocks main execution
- Zero-Config: Works out of box with sensible defaults
"""

import os
import json
import logging
import threading
import time
from datetime import datetime, timezone
from typing import Dict, List, Any, Optional
from dataclasses import dataclass, field

# Check if Redis is available
try:
    import redis
    REDIS_AVAILABLE = True
except ImportError:
    REDIS_AVAILABLE = False

log = logging.getLogger(__name__)

# Configuration from environment
REDIS_URL = os.getenv('REDIS_URL', 'redis://localhost:12963/0')  # Changed default to 12963 (Redis 8.2.2, non-default port for security)
LISTENER_ENABLED = os.getenv('AWARENESS_LISTENER_ENABLED', 'true').lower() == 'true'
CACHE_MAX_SIZE = int(os.getenv('AWARENESS_CACHE_MAX_SIZE', '100'))  # Max events to cache
RECONNECT_DELAY_SECONDS = int(os.getenv('REDIS_RECONNECT_DELAY', '5'))


@dataclass
class AwarenessEvent:
    """
    Single awareness event (message, pheromone, task completion, etc.)
    """
    event_type: str  # 'message', 'pheromone', 'task_complete', 'dm', etc.
    channel: str
    payload: Dict[str, Any]
    timestamp: datetime = field(default_factory=lambda: datetime.now(timezone.utc))

    def to_dict(self) -> Dict[str, Any]:
        """Serialize event to dictionary"""
        return {
            'event_type': self.event_type,
            'channel': self.channel,
            'payload': self.payload,
            'timestamp': self.timestamp.isoformat()
        }


@dataclass
class AwarenessCache:
    """
    Thread-safe cache of new events since last read
    """
    messages: List[AwarenessEvent] = field(default_factory=list)
    pheromones: List[AwarenessEvent] = field(default_factory=list)
    task_completions: List[AwarenessEvent] = field(default_factory=list)
    presence_updates: List[AwarenessEvent] = field(default_factory=list)
    other: List[AwarenessEvent] = field(default_factory=list)

    def add_event(self, event: AwarenessEvent):
        """Add event to appropriate category"""
        if event.event_type == 'message':
            self.messages.append(event)
        elif event.event_type == 'pheromone':
            self.pheromones.append(event)
        elif event.event_type == 'task_complete':
            self.task_completions.append(event)
        elif event.event_type == 'presence':
            self.presence_updates.append(event)
        else:
            self.other.append(event)

    def clear(self):
        """Clear all cached events"""
        self.messages.clear()
        self.pheromones.clear()
        self.task_completions.clear()
        self.presence_updates.clear()
        self.other.clear()

    def is_empty(self) -> bool:
        """Check if cache has any events"""
        return (not self.messages and not self.pheromones and
                not self.task_completions and not self.presence_updates and
                not self.other)

    def total_events(self) -> int:
        """Count total cached events"""
        return (len(self.messages) + len(self.pheromones) +
                len(self.task_completions) + len(self.presence_updates) +
                len(self.other))

    def to_dict(self) -> Dict[str, List[Dict]]:
        """Serialize cache to dictionary"""
        return {
            'messages': [e.to_dict() for e in self.messages],
            'pheromones': [e.to_dict() for e in self.pheromones],
            'task_completions': [e.to_dict() for e in self.task_completions],
            'presence_updates': [e.to_dict() for e in self.presence_updates],
            'other': [e.to_dict() for e in self.other]
        }


class RedisAwarenessListener:
    """
    Background listener for Redis pub/sub events.

    Subscribes to teambook:* pattern and caches all incoming events
    in a thread-safe manner for later retrieval.
    """

    def __init__(self, redis_url: str = None):
        """
        Initialize listener (does not start thread).

        Args:
            redis_url: Redis connection URL (default: from env REDIS_URL)
        """
        self.redis_url = redis_url or REDIS_URL
        self.cache = AwarenessCache()
        self.cache_lock = threading.Lock()

        self.listener_thread: Optional[threading.Thread] = None
        self.stop_event = threading.Event()
        self.running = False

        self.redis_client: Optional[redis.Redis] = None
        self.pubsub: Optional[redis.client.PubSub] = None

        # Statistics
        self.total_received = 0
        self.total_errors = 0
        self.started_at: Optional[datetime] = None

    def start(self):
        """
        Start background listener thread.

        Thread-safe: Can be called multiple times, only starts once.
        """
        if self.running:
            log.debug("Listener already running")
            return

        if not REDIS_AVAILABLE:
            log.warning("Redis library not available, awareness listener disabled")
            return

        if not LISTENER_ENABLED:
            log.info("Awareness listener disabled via config")
            return

        try:
            # Test Redis connection
            test_client = redis.Redis.from_url(self.redis_url, decode_responses=True)
            test_client.ping()
            test_client.close()

            # Start listener thread
            self.stop_event.clear()
            self.listener_thread = threading.Thread(
                target=self._listener_loop,
                name="RedisAwarenessListener",
                daemon=True
            )
            self.listener_thread.start()
            self.running = True
            self.started_at = datetime.now(timezone.utc)

            log.info(f"RedisAwarenessListener started, subscribed to teambook:*")

        except Exception as e:
            log.error(f"Failed to start awareness listener: {e}")
            self.running = False

    def stop(self, timeout: float = 5.0):
        """
        Stop background listener thread gracefully.

        Args:
            timeout: Max seconds to wait for thread to stop
        """
        if not self.running:
            return

        log.info("Stopping RedisAwarenessListener...")
        self.stop_event.set()

        if self.listener_thread and self.listener_thread.is_alive():
            self.listener_thread.join(timeout=timeout)

        # Cleanup Redis connections
        if self.pubsub:
            try:
                self.pubsub.close()
            except:
                pass

        if self.redis_client:
            try:
                self.redis_client.close()
            except:
                pass

        self.running = False
        log.info("RedisAwarenessListener stopped")

    def get_new_info(self) -> Optional[Dict[str, List[Dict]]]:
        """
        Get all new events since last call (atomically reads and clears cache).

        Thread-safe: Uses lock to ensure atomic read-and-clear.

        Returns:
            Dictionary with categorized events, or None if no new events.
            Format: {
                'messages': [...],
                'pheromones': [...],
                'task_completions': [...],
                'presence_updates': [...],
                'other': [...]
            }
        """
        with self.cache_lock:
            if self.cache.is_empty():
                return None

            # Serialize and clear atomically
            result = self.cache.to_dict()
            self.cache.clear()

            return result

    def get_status(self) -> Dict[str, Any]:
        """
        Get listener status and statistics.

        Returns:
            Status dictionary with running state, uptime, event counts, etc.
        """
        uptime_seconds = 0
        if self.started_at:
            uptime_seconds = (datetime.now(timezone.utc) - self.started_at).total_seconds()

        with self.cache_lock:
            cached_events = self.cache.total_events()

        return {
            'running': self.running,
            'redis_available': REDIS_AVAILABLE,
            'enabled': LISTENER_ENABLED,
            'uptime_seconds': uptime_seconds,
            'total_received': self.total_received,
            'total_errors': self.total_errors,
            'cached_events': cached_events,
            'redis_url': self.redis_url.split('@')[-1] if '@' in self.redis_url else self.redis_url  # Hide credentials
        }

    def _listener_loop(self):
        """
        Main listener loop (runs in background thread).

        Subscribes to Redis channels, receives messages, updates cache.
        Auto-reconnects on failures with exponential backoff.
        """
        consecutive_failures = 0
        max_consecutive_failures = 5

        while not self.stop_event.is_set():
            try:
                # Connect to Redis
                self.redis_client = redis.Redis.from_url(
                    self.redis_url,
                    decode_responses=True,
                    socket_keepalive=True,
                    socket_connect_timeout=5,
                    socket_timeout=None  # Block indefinitely waiting for messages (normal for pub/sub)
                )

                # Create pub/sub and subscribe to pattern
                self.pubsub = self.redis_client.pubsub()
                self.pubsub.psubscribe('teambook:*')

                log.info("Connected to Redis, listening for events...")
                consecutive_failures = 0

                # Listen for messages
                for message in self.pubsub.listen():
                    if self.stop_event.is_set():
                        break

                    # Process message
                    try:
                        self._process_message(message)
                    except Exception as e:
                        log.error(f"Error processing message: {e}")
                        self.total_errors += 1

            except redis.ConnectionError as e:
                consecutive_failures += 1
                self.total_errors += 1

                if consecutive_failures >= max_consecutive_failures:
                    log.error(f"Too many consecutive Redis failures ({consecutive_failures}), giving up")
                    break

                log.warning(f"Redis connection lost: {e}, reconnecting in {RECONNECT_DELAY_SECONDS}s...")
                time.sleep(RECONNECT_DELAY_SECONDS)

            except redis.TimeoutError:
                # Socket read timeout is EXPECTED - Redis blocks waiting for messages
                # Don't log as error, don't increment error counter
                # This happens when no messages arrive within socket_timeout
                continue

            except Exception as e:
                # Only log truly unexpected errors
                log.error(f"Unexpected error in listener loop: {e}")
                self.total_errors += 1
                time.sleep(RECONNECT_DELAY_SECONDS)

        # Cleanup on exit
        log.info("Listener loop exited")

    def _process_message(self, message: Dict[str, Any]):
        """
        Process incoming Redis message and add to cache.

        Args:
            message: Raw Redis pub/sub message
        """
        # Skip non-message types (subscribe confirmations, etc.)
        if message['type'] not in ['pmessage', 'message']:
            return

        try:
            # Extract channel and data
            channel = message.get('channel', '')
            data_str = message.get('data', '{}')

            # Parse JSON payload
            try:
                payload = json.loads(data_str) if isinstance(data_str, str) else data_str
            except json.JSONDecodeError:
                # Not JSON, treat as string
                payload = {'raw': data_str}

            # Determine event type from channel
            event_type = self._infer_event_type(channel, payload)

            # Create event
            event = AwarenessEvent(
                event_type=event_type,
                channel=channel,
                payload=payload
            )

            # Add to cache (thread-safe)
            with self.cache_lock:
                # Check cache size limit
                if self.cache.total_events() >= CACHE_MAX_SIZE:
                    log.warning(f"Awareness cache full ({CACHE_MAX_SIZE} events), dropping oldest")
                    # Clear oldest category
                    if self.cache.messages:
                        self.cache.messages.pop(0)
                    elif self.cache.pheromones:
                        self.cache.pheromones.pop(0)
                    elif self.cache.other:
                        self.cache.other.pop(0)

                self.cache.add_event(event)

            self.total_received += 1
            log.debug(f"Received {event_type} event from {channel}")

        except Exception as e:
            log.error(f"Failed to process message: {e}")
            self.total_errors += 1

    def _infer_event_type(self, channel: str, payload: Dict[str, Any]) -> str:
        """
        Infer event type from channel name and payload.

        Args:
            channel: Redis channel name (e.g., 'teambook:messages')
            payload: Event payload

        Returns:
            Event type string ('message', 'pheromone', 'task_complete', etc.)
        """
        # Extract channel suffix
        if ':' in channel:
            suffix = channel.split(':', 1)[1]
        else:
            suffix = channel

        # Map channel patterns to event types
        if 'message' in suffix:
            return 'message'
        elif 'pheromone' in suffix:
            return 'pheromone'
        elif 'task' in suffix and payload.get('status') == 'completed':
            return 'task_complete'
        elif 'presence' in suffix:
            return 'presence'
        elif 'dm' in suffix or 'direct' in suffix:
            return 'message'  # Treat DMs as messages
        else:
            return suffix  # Use channel suffix as event type


# Global listener instance
_listener: Optional[RedisAwarenessListener] = None
_listener_lock = threading.Lock()


def get_listener() -> RedisAwarenessListener:
    """
    Get or create global listener instance (singleton pattern).

    Thread-safe: Uses lock to ensure only one instance created.

    Returns:
        Global RedisAwarenessListener instance
    """
    global _listener

    with _listener_lock:
        if _listener is None:
            _listener = RedisAwarenessListener()
        return _listener


def start_listener():
    """
    Start global awareness listener (convenience function).

    Safe to call multiple times - only starts once.
    """
    listener = get_listener()
    listener.start()


def stop_listener(timeout: float = 5.0):
    """
    Stop global awareness listener (convenience function).

    Args:
        timeout: Max seconds to wait for graceful shutdown
    """
    global _listener

    if _listener:
        _listener.stop(timeout=timeout)


def get_new_awareness_info() -> Optional[Dict[str, List[Dict]]]:
    """
    Get new awareness events from global listener (convenience function).

    Returns:
        Dictionary of categorized events, or None if no new events
    """
    listener = get_listener()
    return listener.get_new_info()


def get_listener_status() -> Dict[str, Any]:
    """
    Get global listener status (convenience function).

    Returns:
        Status dictionary with running state, statistics, etc.
    """
    listener = get_listener()
    return listener.get_status()


# Note: Listener is started by tools/__init__.py on module import
# This avoids double-start and race conditions
