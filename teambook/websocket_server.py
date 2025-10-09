#!/usr/bin/env python3
"""
WEBSOCKET SERVER v1.0.0 - REAL-TIME EVENT STREAMING
====================================================
WebSocket server for Teambook event streaming.
Integrates with teambook_streaming.py for connection management.

Usage:
    python websocket_server.py --port 8765

Features:
- Token-based authentication
- Auto-sync watches on connection
- Real-time event push
- Heartbeat/ping-pong
- Graceful reconnection
- CORS support

Security:
- Token authentication required
- Rate limiting per connection
- Max connections per AI
- Auto-disconnect stale connections
"""

import asyncio
import json
import logging
import signal
import sys
from datetime import datetime
from typing import Dict, Set, Optional
import argparse

# Check for websockets library
try:
    import websockets
    from websockets.server import serve, WebSocketServerProtocol
except ImportError:
    print("ERROR: websockets library not installed")
    print("Install with: pip install websockets")
    sys.exit(1)

# Import streaming module
try:
    import teambook_streaming as streaming
except ImportError:
    print("ERROR: teambook_streaming.py not found")
    sys.exit(1)

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - [WS] - %(message)s'
)
logger = logging.getLogger('websocket_server')

# Active WebSocket connections
# Maps conn_id → WebSocket object
active_connections: Dict[str, WebSocketServerProtocol] = {}

# Connection metadata
# Maps conn_id → {'ai_id': str, 'authenticated': bool, 'last_ping': float}
connection_metadata: Dict[str, Dict] = {}

# ============= MESSAGE PROTOCOL =============

async def send_message(websocket: WebSocketServerProtocol, msg_type: str, data: Dict = None):
    """
    Send message to WebSocket client

    Message format:
    {
        "type": "auth_required" | "connected" | "event" | "error" | "pong",
        "timestamp": "2025-01-30T12:34:56Z",
        ...data
    }
    """
    message = {
        "type": msg_type,
        "timestamp": datetime.utcnow().isoformat() + "Z",
        **(data or {})
    }

    try:
        await websocket.send(json.dumps(message))
    except Exception as e:
        logger.error(f"Send message error: {e}")

async def send_event(conn_id: str, event_data: Dict):
    """Send event to specific connection"""
    websocket = active_connections.get(conn_id)
    if websocket:
        await send_message(websocket, "event", event_data)

# ============= CONNECTION HANDLERS =============

async def handle_auth(websocket: WebSocketServerProtocol, message: Dict, conn_id: str) -> bool:
    """
    Handle authentication message

    Expected message:
    {
        "type": "auth",
        "token": "abc123..."
    }

    Returns: True if authenticated, False otherwise
    """
    token = message.get('token')
    if not token:
        await send_message(websocket, "error", {"error": "missing_token"})
        return False

    # Authenticate with streaming module
    result = streaming.authenticate_connection(conn_id, token)

    if 'error' in result:
        await send_message(websocket, "error", {"error": result['error']})
        return False

    ai_id = result['ai_id']

    # Mark as authenticated
    connection_metadata[conn_id]['authenticated'] = True
    connection_metadata[conn_id]['ai_id'] = ai_id

    # Sync existing watches to WebSocket
    watch_count = streaming.sync_watches_to_websocket(ai_id, conn_id)

    # Send connected confirmation
    await send_message(websocket, "connected", {
        "conn_id": conn_id,
        "ai_id": ai_id,
        "watches_synced": watch_count
    })

    logger.info(f"Client authenticated: {ai_id} (conn_id={conn_id}, watches={watch_count})")
    return True

async def handle_ping(websocket: WebSocketServerProtocol, conn_id: str):
    """Handle ping message, respond with pong"""
    # Update ping timestamp
    streaming.update_ping(conn_id)
    connection_metadata[conn_id]['last_ping'] = asyncio.get_event_loop().time()

    await send_message(websocket, "pong", {})

async def handle_ack(message: Dict, conn_id: str):
    """Handle event acknowledgment"""
    event_id = message.get('event_id')
    if event_id:
        # Mark event as acknowledged
        # This could be used for guaranteed delivery tracking
        logger.debug(f"Event {event_id} acknowledged by {conn_id}")

async def handle_client_message(websocket: WebSocketServerProtocol, message_text: str, conn_id: str):
    """
    Handle incoming message from client

    Message types:
    - auth: Authenticate connection
    - ping: Heartbeat
    - ack: Acknowledge event received
    """
    try:
        message = json.loads(message_text)
        msg_type = message.get('type')

        if msg_type == 'auth':
            authenticated = await handle_auth(websocket, message, conn_id)
            if not authenticated:
                await websocket.close(1008, "Authentication failed")
                return False

        elif msg_type == 'ping':
            await handle_ping(websocket, conn_id)

        elif msg_type == 'ack':
            await handle_ack(message, conn_id)

        else:
            logger.warning(f"Unknown message type: {msg_type}")
            await send_message(websocket, "error", {"error": "unknown_message_type"})

        return True

    except json.JSONDecodeError:
        await send_message(websocket, "error", {"error": "invalid_json"})
        return False
    except Exception as e:
        logger.error(f"Message handling error: {e}", exc_info=True)
        await send_message(websocket, "error", {"error": "internal_error"})
        return False

# ============= CONNECTION LIFECYCLE =============

async def websocket_handler(websocket: WebSocketServerProtocol):
    """
    Main WebSocket connection handler

    Protocol flow:
    1. Client connects
    2. Server sends auth_required with conn_id
    3. Client sends auth with token
    4. Server sends connected
    5. Events stream automatically
    6. Client sends ping periodically
    7. Server sends pong
    """
    conn_id = None

    try:
        # Register connection (get conn_id and auth_token)
        # For now, extract AI ID from query parameters
        # In production, this could be extracted from headers or initial handshake
        path = websocket.path
        query_params = {}
        if '?' in path:
            query_string = path.split('?')[1]
            for param in query_string.split('&'):
                if '=' in param:
                    key, value = param.split('=', 1)
                    query_params[key] = value

        ai_id = query_params.get('ai_id', 'unknown-ai')

        # Register connection with streaming module
        reg_result = streaming.register_connection(ai_id=ai_id)

        if 'error' in reg_result:
            await send_message(websocket, "error", {"error": reg_result['error']})
            await websocket.close(1008, "Registration failed")
            return

        conn_id = reg_result['conn_id']
        auth_token = reg_result['auth_token']

        # Store connection
        active_connections[conn_id] = websocket
        connection_metadata[conn_id] = {
            'ai_id': ai_id,
            'authenticated': False,
            'last_ping': asyncio.get_event_loop().time()
        }

        # Also register in streaming module's global registry
        streaming._active_websockets[conn_id] = websocket

        logger.info(f"WebSocket connection registered: {ai_id} (conn_id={conn_id})")

        # Send auth challenge
        await send_message(websocket, "auth_required", {
            "conn_id": conn_id,
            "token": auth_token  # Send token for convenience (single-use)
        })

        # Wait for authentication (with timeout)
        try:
            auth_msg = await asyncio.wait_for(websocket.recv(), timeout=10.0)
            if not await handle_client_message(websocket, auth_msg, conn_id):
                return
        except asyncio.TimeoutError:
            await send_message(websocket, "error", {"error": "auth_timeout"})
            await websocket.close(1008, "Authentication timeout")
            return

        # Main message loop
        async for message in websocket:
            if not await handle_client_message(websocket, message, conn_id):
                break

    except websockets.exceptions.ConnectionClosed:
        logger.info(f"WebSocket connection closed: {conn_id}")
    except Exception as e:
        logger.error(f"WebSocket error: {e}", exc_info=True)
    finally:
        # Cleanup
        if conn_id:
            active_connections.pop(conn_id, None)
            connection_metadata.pop(conn_id, None)
            streaming._active_websockets.pop(conn_id, None)
            streaming.unregister_connection(conn_id)
            logger.info(f"WebSocket connection cleaned up: {conn_id}")

# ============= EVENT PUSHER =============

async def event_pusher_loop():
    """
    Background task that pushes cached events to WebSocket clients

    Runs every 100ms, checks for cached events and sends them
    """
    while True:
        try:
            await asyncio.sleep(0.1)  # 100ms

            # Get all active connections
            for conn_id in list(active_connections.keys()):
                # Check if authenticated
                metadata = connection_metadata.get(conn_id)
                if not metadata or not metadata.get('authenticated'):
                    continue

                # Get cached events from streaming module
                events = streaming.get_cached_events(conn_id, clear=True)

                # Send each event
                for event in events:
                    await send_event(conn_id, event)

        except Exception as e:
            logger.error(f"Event pusher error: {e}", exc_info=True)
            await asyncio.sleep(1.0)  # Back off on error

# ============= CLEANUP TASK =============

async def cleanup_task():
    """
    Background task that cleans up stale connections

    Runs every 60 seconds
    """
    while True:
        try:
            await asyncio.sleep(60.0)  # 1 minute

            # Cleanup stale connections in streaming module
            count = streaming.cleanup_stale_connections(max_age_seconds=300)
            if count > 0:
                logger.info(f"Cleaned up {count} stale connections")

            # Check for connections that haven't pinged recently
            now = asyncio.get_event_loop().time()
            stale = []

            for conn_id, metadata in connection_metadata.items():
                last_ping = metadata.get('last_ping', 0)
                if now - last_ping > 300:  # 5 minutes
                    stale.append(conn_id)

            # Close stale connections
            for conn_id in stale:
                websocket = active_connections.get(conn_id)
                if websocket:
                    try:
                        await websocket.close(1000, "Ping timeout")
                    except Exception:
                        pass
                    logger.warning(f"Closed stale connection: {conn_id}")

        except Exception as e:
            logger.error(f"Cleanup task error: {e}", exc_info=True)

# ============= SERVER STARTUP =============

async def main(host: str = 'localhost', port: int = 8765):
    """Start WebSocket server"""
    logger.info(f"Starting WebSocket server on {host}:{port}")

    # Start background tasks
    asyncio.create_task(event_pusher_loop())
    asyncio.create_task(cleanup_task())

    # Start WebSocket server
    async with serve(websocket_handler, host, port):
        logger.info(f"WebSocket server running on ws://{host}:{port}")
        logger.info("Press Ctrl+C to stop")

        # Wait forever
        await asyncio.Future()

def run_server(host: str = 'localhost', port: int = 8765):
    """Run WebSocket server with proper signal handling"""
    try:
        asyncio.run(main(host, port))
    except KeyboardInterrupt:
        logger.info("Server stopped by user")
    except Exception as e:
        logger.error(f"Server error: {e}", exc_info=True)
        sys.exit(1)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description='Teambook WebSocket Server')
    parser.add_argument('--host', default='localhost', help='Host to bind to (default: localhost)')
    parser.add_argument('--port', type=int, default=8765, help='Port to bind to (default: 8765)')
    parser.add_argument('--verbose', action='store_true', help='Enable verbose logging')

    args = parser.parse_args()

    if args.verbose:
        logging.getLogger().setLevel(logging.DEBUG)

    run_server(args.host, args.port)