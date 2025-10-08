"""
Teambook Health Monitor Server

Flask server providing REST API for health checks and configuration.
Serves the beautiful cyberpunk health monitoring UI.
"""

import os
import sys
import time
import json
import re
from datetime import datetime, timedelta
from flask import Flask, jsonify, request, send_from_directory
from flask_cors import CORS
from urllib.parse import urlparse
import socket

# Add teambook to path (system-agnostic)
current_dir = os.path.dirname(os.path.abspath(__file__))
src_path = os.path.join(current_dir, '..', 'src')
sys.path.insert(0, src_path)

from teambook.storage_adapter import TeambookStorageAdapter
from teambook.teambook_config import get_storage_backend
from teambook import teambook_api

app = Flask(__name__, static_folder='.')
CORS(app)

# Startup time for uptime calculation
START_TIME = datetime.now()

# Performance tracking
perf_stats = {
    'writes': [],
    'reads': [],
    'last_cleanup': datetime.now()
}

# Security: Rate limiting for API endpoints
from collections import defaultdict
_rate_limit_requests = defaultdict(list)
MAX_CONFIG_REQUESTS_PER_MINUTE = 10


def rate_limit_check(client_ip, max_requests=MAX_CONFIG_REQUESTS_PER_MINUTE):
    """
    Check if client has exceeded rate limit.

    Args:
        client_ip: Client IP address
        max_requests: Maximum requests per minute

    Returns:
        True if within limit, False if exceeded
    """
    from time import time

    now = time()
    # Clean old entries (older than 60 seconds)
    _rate_limit_requests[client_ip] = [
        t for t in _rate_limit_requests[client_ip] if now - t < 60
    ]

    # Check if limit exceeded
    if len(_rate_limit_requests[client_ip]) >= max_requests:
        return False

    # Add current request
    _rate_limit_requests[client_ip].append(now)
    return True


def validate_database_url(url, allowed_schemes=None):
    """
    Validate database URL to prevent SSRF and command injection.

    Args:
        url: Database URL to validate
        allowed_schemes: List of allowed URL schemes (default: postgresql, postgres, redis, rediss)

    Returns:
        Validated URL

    Raises:
        ValueError: If URL is invalid or unsafe
    """
    import ipaddress

    if not url:
        raise ValueError("URL cannot be empty")

    if allowed_schemes is None:
        allowed_schemes = ['postgresql', 'postgres', 'redis', 'rediss']

    # Check for command injection characters
    dangerous_chars = [';', '|', '&', '$', '(', ')', '{', '}', '`', '\n', '\r']
    if any(char in url for char in dangerous_chars):
        raise ValueError("URL contains invalid characters")

    # Parse URL
    try:
        parsed = urlparse(url)
    except Exception as e:
        raise ValueError(f"Invalid URL format: {e}")

    # Validate scheme
    if parsed.scheme not in allowed_schemes:
        raise ValueError(f"Invalid URL scheme. Allowed: {', '.join(allowed_schemes)}")

    # Validate hostname exists
    if not parsed.hostname:
        raise ValueError("URL must contain a hostname")

    # Block localhost aliases
    localhost_names = ['localhost', 'localhost.localdomain', '0.0.0.0', 'broadcasthost']
    if parsed.hostname.lower() in localhost_names:
        raise ValueError("Localhost connections not allowed for security")

    # Security: Comprehensive SSRF protection with IPv4 and IPv6
    try:
        # Get ALL IP addresses (IPv4 and IPv6)
        addr_info = socket.getaddrinfo(parsed.hostname, None)

        for family, _, _, _, sockaddr in addr_info:
            ip_str = sockaddr[0]

            try:
                ip = ipaddress.ip_address(ip_str)

                # Block all private/reserved/special ranges
                if (ip.is_private or
                    ip.is_loopback or
                    ip.is_link_local or
                    ip.is_multicast or
                    ip.is_reserved or
                    ip.is_unspecified):
                    raise ValueError(f"IP address {ip} is in a blocked range")

                # Explicitly block cloud metadata endpoints (AWS, Azure, GCP, etc.)
                if str(ip).startswith('169.254.'):
                    raise ValueError("Cloud metadata endpoints not allowed")

            except ValueError as e:
                if "is in a blocked range" in str(e) or "not allowed" in str(e):
                    raise
                # Not a valid IP, skip
                continue

    except socket.gaierror:
        # Hostname doesn't resolve - BLOCK IT for security
        raise ValueError(f"Hostname {parsed.hostname} does not resolve to a valid IP address")

    return url


def validate_file_path(path, allowed_base=None):
    """
    Validate file path to prevent path traversal.

    Args:
        path: File path to validate
        allowed_base: Base directory that path must be within

    Returns:
        Validated absolute path

    Raises:
        ValueError: If path is invalid or outside allowed directory
    """
    if not path:
        raise ValueError("Path cannot be empty")

    # Check for dangerous path components
    dangerous_patterns = ['..', '~', '$']
    if any(pattern in path for pattern in dangerous_patterns):
        raise ValueError("Path contains invalid patterns")

    # Resolve to absolute path
    abs_path = os.path.abspath(path)

    # If allowed_base specified, ensure path is within it
    if allowed_base:
        allowed_base = os.path.abspath(allowed_base)
        if not abs_path.startswith(allowed_base):
            raise ValueError("Path outside allowed directory")

    return abs_path


def get_uptime():
    """Calculate uptime since server started!"""
    delta = datetime.now() - START_TIME
    days = delta.days
    hours, remainder = divmod(delta.seconds, 3600)
    minutes, seconds = divmod(remainder, 60)

    if days > 0:
        return f"{days}d {hours}h {minutes}m"
    elif hours > 0:
        return f"{hours}h {minutes}m"
    else:
        return f"{minutes}m {seconds}s"


def test_postgresql():
    """Test PostgreSQL connection and get stats!"""
    try:
        from teambook.teambook_storage_postgresql import PostgreSQLTeambookStorage

        # Check if PostgreSQL URL is configured
        postgres_url = os.environ.get('POSTGRES_URL') or os.environ.get('DATABASE_URL')
        if not postgres_url:
            return {
                'connected': False,
                'error': 'NO POSTGRES_URL SET'
            }

        # Test connection
        start = time.time()
        storage = PostgreSQLTeambookStorage('health-check')
        latency = int((time.time() - start) * 1000)

        # Get note count
        notes = storage.read_notes(limit=1)
        note_count = len(notes)

        # Get pool stats
        pool = storage.pool
        pool_used = pool._used
        pool_max = pool.maxconn

        return {
            'connected': True,
            'latency': latency,
            'noteCount': note_count,
            'poolUsed': len(pool_used),
            'poolMax': pool_max,
            'url': postgres_url.split('@')[1] if '@' in postgres_url else postgres_url  # Hide credentials
        }

    except Exception as e:
        return {
            'connected': False,
            'error': str(e)[:100]
        }


def test_redis():
    """Test Redis connection and get stats!"""
    try:
        import redis

        # Check if Redis is configured
        use_redis = os.environ.get('USE_REDIS', '').lower() == 'true'
        if not use_redis:
            return {
                'connected': False,
                'error': 'USE_REDIS NOT ENABLED'
            }

        redis_url = os.environ.get('REDIS_URL', 'redis://localhost:6379/0')

        # Test connection
        start = time.time()
        r = redis.from_url(redis_url)
        r.ping()
        latency = int((time.time() - start) * 1000)

        # Get memory usage
        info = r.info('memory')
        memory_used = info.get('used_memory_human', 'Unknown')

        # Check pub/sub
        pubsub_active = True  # Always true if connected

        return {
            'connected': True,
            'latency': latency,
            'memoryUsed': memory_used,
            'pubsubActive': pubsub_active,
            'url': redis_url
        }

    except Exception as e:
        return {
            'connected': False,
            'error': str(e)[:100]
        }


def test_duckdb():
    """Test DuckDB connection and get stats!"""
    try:
        from teambook.teambook_storage import TeambookStorage

        # DuckDB is always available
        start = time.time()
        storage = TeambookStorage('health-check')
        latency = int((time.time() - start) * 1000)

        # Get note count
        notes = storage.read_notes(limit=1000000)
        note_count = len(notes)

        # Get database size
        db_path = storage.db_path
        if os.path.exists(db_path):
            size_bytes = os.path.getsize(db_path)
            if size_bytes > 1024 * 1024:
                size_formatted = f"{size_bytes / (1024 * 1024):.2f} MB"
            else:
                size_formatted = f"{size_bytes / 1024:.2f} KB"
        else:
            size_formatted = "Not created"

        return {
            'connected': True,
            'latency': latency,
            'noteCount': note_count,
            'sizeFormatted': size_formatted,
            'path': db_path
        }

    except Exception as e:
        return {
            'connected': False,
            'error': str(e)[:100],
            'noteCount': 0,
            'sizeFormatted': 'Unknown'
        }


def calculate_performance_stats():
    """Calculate recent performance metrics!"""
    global perf_stats

    # Cleanup old entries (keep last 60 seconds)
    now = datetime.now()
    if (now - perf_stats['last_cleanup']).seconds > 10:
        cutoff = now - timedelta(seconds=60)
        perf_stats['writes'] = [t for t in perf_stats['writes'] if t > cutoff]
        perf_stats['reads'] = [t for t in perf_stats['reads'] if t > cutoff]
        perf_stats['last_cleanup'] = now

    # Calculate ops/sec
    writes_per_sec = len(perf_stats['writes'])
    reads_per_sec = len(perf_stats['reads'])

    # Calculate average latency (simulated for now)
    avg_latency = 5  # Will be calculated from actual operations

    return {
        'writesPerSec': writes_per_sec,
        'readsPerSec': reads_per_sec,
        'avgLatency': avg_latency,
        'uptime': get_uptime()
    }


@app.route('/')
def index():
    """Serve the health monitor UI!"""
    return send_from_directory('.', 'teambook_health.html')


@app.route('/health_style.css')
def style():
    """Serve CSS!"""
    return send_from_directory('.', 'health_style.css')


@app.route('/health_script.js')
def script():
    """Serve JavaScript!"""
    return send_from_directory('.', 'health_script.js')


@app.route('/api/health')
def health():
    """Get health status of all backends!"""
    postgresql = test_postgresql()
    redis = test_redis()
    duckdb = test_duckdb()

    # Determine active backend
    active_backend = get_storage_backend()

    # Determine overall health
    if postgresql['connected']:
        overall = 'healthy'
    elif redis['connected']:
        overall = 'degraded'
    elif duckdb['connected']:
        overall = 'degraded'
    else:
        overall = 'error'

    # Get performance stats
    stats = calculate_performance_stats()

    return jsonify({
        'postgresql': postgresql,
        'redis': redis,
        'duckdb': duckdb,
        'activeBackend': active_backend,
        'overall': overall,
        'stats': stats,
        'timestamp': datetime.now().isoformat()
    })


@app.route('/api/test/<backend>')
def test_backend(backend):
    """Test a specific backend connection!"""
    if backend == 'postgresql':
        result = test_postgresql()
    elif backend == 'redis':
        result = test_redis()
    elif backend == 'duckdb':
        result = test_duckdb()
    else:
        return jsonify({'success': False, 'error': 'Unknown backend'}), 400

    return jsonify({
        'success': result['connected'],
        'latency': result.get('latency', 0),
        'noteCount': result.get('noteCount', 0),
        'error': result.get('error', None)
    })


@app.route('/api/config/<backend>', methods=['POST'])
def configure_backend(backend):
    """Configure a backend with rate limiting!"""
    # Security: Apply rate limiting
    client_ip = request.remote_addr
    if not rate_limit_check(client_ip):
        return jsonify({'success': False, 'error': 'Rate limit exceeded. Please try again later.'}), 429

    data = request.json

    try:
        if backend == 'postgresql':
            url = data.get('url')
            min_conn = data.get('minConn', 2)
            max_conn = data.get('maxConn', 10)

            # Security: Validate URL to prevent SSRF and command injection
            try:
                url = validate_database_url(url, allowed_schemes=['postgresql', 'postgres'])
            except ValueError as e:
                return jsonify({'success': False, 'error': f'Invalid PostgreSQL URL: {str(e)}'}), 400

            # Validate connection pool parameters
            if not isinstance(min_conn, int) or min_conn < 1 or min_conn > 100:
                return jsonify({'success': False, 'error': 'minConn must be between 1 and 100'}), 400
            if not isinstance(max_conn, int) or max_conn < 1 or max_conn > 100:
                return jsonify({'success': False, 'error': 'maxConn must be between 1 and 100'}), 400

            # Set environment variables
            os.environ['POSTGRES_URL'] = url
            os.environ['POSTGRES_MIN_CONN'] = str(min_conn)
            os.environ['POSTGRES_MAX_CONN'] = str(max_conn)

            # Test connection
            result = test_postgresql()

            if result['connected']:
                return jsonify({'success': True, 'message': 'PostgreSQL configured successfully'})
            else:
                return jsonify({'success': False, 'error': result['error']}), 400

        elif backend == 'redis':
            url = data.get('url')
            pubsub = data.get('pubsub', True)

            # Security: Validate URL to prevent SSRF and command injection
            try:
                url = validate_database_url(url, allowed_schemes=['redis', 'rediss'])
            except ValueError as e:
                return jsonify({'success': False, 'error': f'Invalid Redis URL: {str(e)}'}), 400

            # Set environment variables
            os.environ['REDIS_URL'] = url
            os.environ['USE_REDIS'] = 'true' if pubsub else 'false'

            # Test connection
            result = test_redis()

            if result['connected']:
                return jsonify({'success': True, 'message': 'Redis configured successfully'})
            else:
                return jsonify({'success': False, 'error': result['error']}), 400

        elif backend == 'duckdb':
            path = data.get('path', '')
            readonly = data.get('readonly', False)

            # Security: Validate file path to prevent path traversal
            if path:
                try:
                    path = validate_file_path(path)
                except ValueError as e:
                    return jsonify({'success': False, 'error': f'Invalid DuckDB path: {str(e)}'}), 400

            # Set environment variables
            if path:
                os.environ['DUCKDB_PATH'] = path
            os.environ['DUCKDB_READONLY'] = 'true' if readonly else 'false'

            return jsonify({'success': True, 'message': 'DuckDB configured successfully'})

        else:
            return jsonify({'success': False, 'error': 'Unknown backend'}), 400

    except Exception as e:
        return jsonify({'success': False, 'error': str(e)}), 500


@app.route('/api/stats', methods=['POST'])
def record_stat():
    """Record a performance stat (write or read)!"""
    data = request.json
    op_type = data.get('type')

    if op_type == 'write':
        perf_stats['writes'].append(datetime.now())
    elif op_type == 'read':
        perf_stats['reads'].append(datetime.now())

    return jsonify({'success': True})


@app.route('/api/ai-network')
def ai_network():
    """Get REAL AI network status from actual teambook data!"""
    try:
        # Use the CURRENT teambook (system-agnostic)
        current_teambook = os.environ.get('TEAMBOOK_NAME', 'town-hall-qd')

        # Get storage adapter for current teambook
        storage = TeambookStorageAdapter(current_teambook)

        # Get recent notes (last 200 to see activity patterns)
        recent_notes = storage.read_notes(limit=200)

        # Track REAL AI activity from actual notes
        ai_activity = {}
        ai_connections = {}  # Track who sends to whom

        for note in recent_notes:
            owner = note.get('owner', 'unknown')

            # Initialize if new AI
            if owner not in ai_activity:
                ai_activity[owner] = {
                    'sent': 0,
                    'received': 0,
                    'lastCommand': None,
                    'lastSeen': note.get('created', ''),
                    'connections': {}
                }

            # Count sent messages
            ai_activity[owner]['sent'] += 1

            # Get last command (from summary or content)
            if not ai_activity[owner]['lastCommand']:
                summary = note.get('summary', '')
                content = note.get('content', '')
                ai_activity[owner]['lastCommand'] = summary[:100] if summary else content[:100]

            # Track connections (who this AI mentions/interacts with)
            content_lower = note.get('content', '').lower()
            for other_ai in ai_activity.keys():
                if other_ai != owner and other_ai in content_lower:
                    if other_ai not in ai_activity[owner]['connections']:
                        ai_activity[owner]['connections'][other_ai] = 0
                    ai_activity[owner]['connections'][other_ai] += 1

        # Build nodes with REAL data
        nodes = []
        ai_list = list(ai_activity.keys())

        # Teambook at center (hub model as requested)
        teambook_node = {
            'id': 'teambook',
            'name': current_teambook.upper(),
            'status': 'active',
            'lastCommand': 'Central coordination hub',
            'sent': sum(a['sent'] for a in ai_activity.values()),
            'received': sum(a['sent'] for a in ai_activity.values()),
            'x': 0.5,  # Center
            'y': 0.5
        }
        nodes.append(teambook_node)

        # Position other AIs in circle around teambook
        import math
        num_ais = len(ai_list)
        radius = 0.35  # Distance from center

        for idx, ai_id in enumerate(ai_list):
            activity = ai_activity[ai_id]

            # Calculate position in circle around center
            angle = (2 * math.pi * idx) / num_ais
            x = 0.5 + radius * math.cos(angle)
            y = 0.5 + radius * math.sin(angle)

            # Determine REAL status based on recent activity
            last_seen = activity['lastSeen']
            if isinstance(last_seen, str):
                try:
                    last_seen_dt = datetime.fromisoformat(last_seen.replace('Z', '+00:00'))
                    age_minutes = (datetime.now() - last_seen_dt.replace(tzinfo=None)).total_seconds() / 60
                except:
                    age_minutes = 999
            else:
                age_minutes = 999

            # Real status based on actual activity time
            if age_minutes < 10:
                status = 'active'
            elif age_minutes < 60:
                status = 'idle'
            else:
                status = 'offline'

            nodes.append({
                'id': ai_id,
                'name': ai_id.upper(),
                'status': status,
                'lastCommand': activity['lastCommand'] or 'No recent activity',
                'sent': activity['sent'],
                'received': sum(ai_activity.get(other, {}).get('connections', {}).get(ai_id, 0) for other in ai_list),
                'x': x,
                'y': y
            })

        # Build REAL connections based on actual interactions
        connections = []

        # All AIs connect to teambook (hub model)
        for node in nodes:
            if node['id'] != 'teambook':
                connections.append({
                    'from': node['id'],
                    'to': 'teambook',
                    'strength': min(node['sent'] / 20.0, 1.0)  # Based on real activity
                })

        # Add peer-to-peer connections based on real interactions
        for ai_id, activity in ai_activity.items():
            for other_ai, count in activity['connections'].items():
                if count > 0:
                    connections.append({
                        'from': ai_id,
                        'to': other_ai,
                        'strength': min(count / 10.0, 1.0)  # Based on real mentions
                    })

        return jsonify({
            'nodes': nodes,
            'connections': connections,
            'timestamp': datetime.now().isoformat(),
            'teambook': current_teambook
        })

    except Exception as e:
        # Return error info (no fake data!)
        print(f"ERROR fetching AI network: {e}")
        import traceback
        traceback.print_exc()

        return jsonify({
            'nodes': [],
            'connections': [],
            'error': str(e),
            'timestamp': datetime.now().isoformat()
        }), 500


def main():
    """Start the health monitor server!"""
    print("=" * 60)
    print("TEAMBOOK HEALTH MONITOR")
    print("=" * 60)
    print("")
    print("Starting server on http://localhost:8765")
    print("")
    print("Open http://localhost:8765 in your browser to view the health dashboard")
    print("")
    print("Press Ctrl+C to stop")
    print("")
    print("=" * 60)

    app.run(host='0.0.0.0', port=8765, debug=False)


if __name__ == '__main__':
    main()
