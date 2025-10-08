"""
Teambook configuration management.

Handles switching between PostgreSQL, Redis, and DuckDB backends with priority fallback.
"""

import os
from typing import Literal

StorageBackend = Literal["postgresql", "redis", "duckdb"]


def get_storage_backend() -> StorageBackend:
    """
    Get the configured storage backend with automatic fallback.

    Priority order:
    1. PostgreSQL (if POSTGRES_URL or DATABASE_URL set, psycopg2 available, AND connectable)
    2. Redis (if USE_REDIS=true, redis available, AND connectable)
    3. DuckDB (always available as fallback)

    Returns:
        "postgresql", "redis", or "duckdb"

    Note: This function now VERIFIES connectivity, not just configuration.
    """
    import logging

    # Try PostgreSQL first
    postgres_url = os.getenv('POSTGRES_URL') or os.getenv('DATABASE_URL')
    if postgres_url:
        try:
            import psycopg2
            # CRITICAL FIX: Actually test connectivity, don't just check if psycopg2 exists!
            conn = psycopg2.connect(postgres_url, connect_timeout=2)
            conn.close()
            return "postgresql"
        except ImportError:
            logging.warning("PostgreSQL URL configured but psycopg2 not installed. Falling back to Redis/DuckDB.")
        except Exception as e:
            logging.warning(f"PostgreSQL URL configured but connection failed: {e}. Falling back to Redis/DuckDB.")

    # Try Redis second
    use_redis = os.getenv('USE_REDIS', 'false').lower() in ('true', '1', 'yes')
    if use_redis:
        try:
            import redis
            # CRITICAL FIX: Actually test connectivity!
            r = redis.Redis.from_url(os.getenv('REDIS_URL', 'redis://localhost:6379'), socket_connect_timeout=2)
            r.ping()
            return "redis"
        except ImportError:
            logging.warning("Redis enabled but 'redis' package not installed. Falling back to DuckDB.")
        except Exception as e:
            logging.warning(f"Redis enabled but connection failed: {e}. Falling back to DuckDB.")

    # Fallback to DuckDB (always available)
    return "duckdb"


def use_postgresql() -> bool:
    """Check if PostgreSQL backend is enabled."""
    return get_storage_backend() == "postgresql"


def use_redis() -> bool:
    """Check if Redis backend is enabled."""
    return get_storage_backend() == "redis"


def use_duckdb() -> bool:
    """Check if DuckDB backend is enabled."""
    return get_storage_backend() == "duckdb"


# Configuration validation
def validate_config() -> None:
    """Validate configuration and dependencies."""
    backend = get_storage_backend()
    
    if backend == "postgresql":
        postgres_url = os.getenv('POSTGRES_URL') or os.getenv('DATABASE_URL')
        if not postgres_url:
            raise RuntimeError("PostgreSQL backend detected but no POSTGRES_URL or DATABASE_URL set")
        
        try:
            import psycopg2
        except ImportError:
            raise RuntimeError(
                "PostgreSQL backend enabled but 'psycopg2' package not installed. "
                "Install with: pip install psycopg2-binary"
            )
        
        # Check PostgreSQL connectivity
        try:
            import psycopg2
            conn = psycopg2.connect(postgres_url)
            conn.close()
        except Exception as e:
            raise RuntimeError(
                f"PostgreSQL backend enabled but cannot connect to database: {e}. "
                "Make sure PostgreSQL is running and POSTGRES_URL is correct."
            )
    
    elif backend == "redis":
        try:
            import redis
        except ImportError:
            raise RuntimeError(
                "Redis backend enabled but 'redis' package not installed. "
                "Install with: pip install redis>=5.1.0"
            )

        # Check Redis connectivity
        from .redis_pool import get_connection
        try:
            conn = get_connection()
            conn.ping()
        except Exception as e:
            raise RuntimeError(
                f"Redis backend enabled but cannot connect to Redis server: {e}. "
                "Make sure Redis is running and REDIS_URL is correct."
            )
    
    # DuckDB always works, no validation needed
