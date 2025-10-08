"""
Redis pub/sub event system for real-time collaboration.

Enables:
- Real-time message notifications
- Hook triggers
- Wait function coordination
- Instance presence tracking
"""

import json
import threading
import logging
from typing import Callable, Dict, Any, Optional
from datetime import datetime

from .redis_pool import get_connection, get_pubsub

logger = logging.getLogger(__name__)


class TeambookEvents:
    """Manages pub/sub events for a Teambook."""

    def __init__(self, teambook_name: str):
        self.teambook_name = teambook_name
        self.channel = f"teambook:{teambook_name}:events"
        self.pubsub = None
        self.listener_thread = None
        self.handlers: Dict[str, list[Callable]] = {}
        self._running = False

    def publish(self, event_type: str, data: Dict[str, Any]) -> None:
        """
        Publish an event to all instances.

        Args:
            event_type: Type of event (e.g., 'note_created', 'message_broadcast')
            data: Event payload data
        """
        redis_conn = get_connection()

        event = {
            'type': event_type,
            'timestamp': datetime.utcnow().isoformat(),
            'data': data
        }

        message = json.dumps(event)
        redis_conn.publish(self.channel, message)

        logger.debug(f"Published event: {event_type} to {self.channel}")

    def subscribe(self, event_type: str, handler: Callable[[Dict[str, Any]], None]) -> None:
        """
        Subscribe to a specific event type.

        Args:
            event_type: Type of event to listen for
            handler: Callback function that receives event data
        """
        if event_type not in self.handlers:
            self.handlers[event_type] = []

        self.handlers[event_type].append(handler)
        logger.info(f"Subscribed to event type: {event_type}")

    def start_listening(self) -> None:
        """Start listening for events in a background thread."""
        if self._running:
            logger.warning("Event listener already running")
            return

        self.pubsub = get_pubsub(self.channel)
        self._running = True

        self.listener_thread = threading.Thread(
            target=self._listen_loop,
            daemon=True
        )
        self.listener_thread.start()

        logger.info(f"Started event listener for {self.channel}")

    def stop_listening(self) -> None:
        """Stop the event listener."""
        self._running = False

        if self.listener_thread:
            self.listener_thread.join(timeout=2)
            self.listener_thread = None

        if self.pubsub:
            self.pubsub.unsubscribe()
            self.pubsub.close()
            self.pubsub = None

        logger.info(f"Stopped event listener for {self.channel}")

    def _listen_loop(self) -> None:
        """Background loop that processes pub/sub messages."""
        logger.info("Event listener loop started")

        try:
            for message in self.pubsub.listen():
                if not self._running:
                    break

                if message['type'] == 'message':
                    try:
                        event = json.loads(message['data'])
                        self._handle_event(event)
                    except Exception as e:
                        logger.error(f"Error processing event: {e}", exc_info=True)

        except Exception as e:
            logger.error(f"Event listener loop error: {e}", exc_info=True)
        finally:
            logger.info("Event listener loop stopped")

    def _handle_event(self, event: Dict[str, Any]) -> None:
        """
        Dispatch event to registered handlers.

        Args:
            event: Event dictionary with 'type' and 'data' keys
        """
        event_type = event.get('type')
        event_data = event.get('data', {})

        if event_type in self.handlers:
            for handler in self.handlers[event_type]:
                try:
                    handler(event_data)
                except Exception as e:
                    logger.error(f"Error in event handler for {event_type}: {e}", exc_info=True)


# Global event managers for each teambook
_event_managers: Dict[str, TeambookEvents] = {}


def get_events(teambook_name: str) -> TeambookEvents:
    """
    Get or create an event manager for a teambook.

    Args:
        teambook_name: Name of the teambook

    Returns:
        TeambookEvents instance
    """
    if teambook_name not in _event_managers:
        _event_managers[teambook_name] = TeambookEvents(teambook_name)

    return _event_managers[teambook_name]


def publish_event(teambook_name: str, event_type: str, data: Dict[str, Any]) -> None:
    """
    Convenience function to publish an event.

    Args:
        teambook_name: Name of the teambook
        event_type: Type of event
        data: Event payload
    """
    events = get_events(teambook_name)
    events.publish(event_type, data)


def subscribe_event(teambook_name: str, event_type: str, handler: Callable) -> None:
    """
    Convenience function to subscribe to an event.

    Args:
        teambook_name: Name of the teambook
        event_type: Type of event
        handler: Callback function
    """
    events = get_events(teambook_name)
    events.subscribe(event_type, handler)

    # Auto-start listener if not running
    if not events._running:
        events.start_listening()
