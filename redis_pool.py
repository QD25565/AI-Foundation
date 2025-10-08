"""
Redis connection pooling for Teambook.

Provides thread-safe connection pooling and pub/sub management for multi-instance collaboration.
"""

import os
import redis
from redis.connection import ConnectionPool
from typing import Optional
import logging

logger = logging.getLogger(__name__)

# Global connection pool (initialized once)
_pool: Optional[ConnectionPool] = None
_pubsub_connections = {}


def get_redis_url() -> str:
    """Get Redis URL from environment or use default."""
    return os.getenv('REDIS_URL', 'redis://localhost:6379/0')


def init_pool(url: Optional[str] = None) -> ConnectionPool:
    """
    Initialize the Redis connection pool.

    Args:
        url: Redis connection URL (defaults to REDIS_URL env var or localhost)

    Returns:
        ConnectionPool instance
    """
    global _pool

    if _pool is not None:
        return _pool

    redis_url = url or get_redis_url()

    _pool = ConnectionPool.from_url(
        redis_url,
        max_connections=20,
        decode_responses=True,  # Auto-decode bytes to strings
        socket_keepalive=True,
        socket_connect_timeout=5,
        retry_on_timeout=True
    )

    logger.info(f"Redis connection pool initialized: {redis_url}")
    return _pool


def get_connection() -> redis.Redis:
    """
    Get a Redis connection from the pool.

    Returns:
        Redis client instance
    """
    global _pool

    if _pool is None:
        init_pool()

    return redis.Redis(connection_pool=_pool)


def get_pubsub(channel: str) -> redis.client.PubSub:
    """
    Get a dedicated pub/sub connection for a channel.

    Pub/sub connections are long-lived and should not be returned to the pool.
    Each channel gets its own connection.

    Args:
        channel: Pub/sub channel name

    Returns:
        PubSub instance subscribed to the channel
    """
    global _pubsub_connections

    if channel in _pubsub_connections:
        return _pubsub_connections[channel]

    # Create dedicated connection for pub/sub (not from pool)
    redis_url = get_redis_url()
    conn = redis.Redis.from_url(
        redis_url,
        decode_responses=True,
        socket_keepalive=True
    )

    pubsub = conn.pubsub()
    pubsub.subscribe(channel)

    _pubsub_connections[channel] = pubsub
    logger.info(f"Created pub/sub connection for channel: {channel}")

    return pubsub


def close_all():
    """Close all connections and clean up resources."""
    global _pool, _pubsub_connections

    # Close pub/sub connections
    for channel, pubsub in _pubsub_connections.items():
        try:
            pubsub.unsubscribe()
            pubsub.close()
            logger.info(f"Closed pub/sub connection for channel: {channel}")
        except Exception as e:
            logger.error(f"Error closing pub/sub for {channel}: {e}")

    _pubsub_connections.clear()

    # Disconnect pool
    if _pool is not None:
        _pool.disconnect()
        logger.info("Redis connection pool closed")
        _pool = None


# Register cleanup on exit
import atexit
atexit.register(close_all)
