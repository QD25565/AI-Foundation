"""
Teambook Health Monitor Server - v2 with WebSocket Support

REAL backend integration - NO MOCK DATA!
"""

import os
import sys
import time
import json
import asyncio
from datetime import datetime, timedelta
from flask import Flask, jsonify, request, send_from_directory
from flask_cors import CORS
from flask_sock import Sock

# Add teambook to path (system-agnostic)
current_dir = os.path.dirname(os.path.abspath(__file__))
src_path = os.path.join(current_dir, '..', 'src')
sys.path.insert(0, src_path)

from teambook.storage_adapter import TeambookStorageAdapter
from teambook.teambook_config import get_storage_backend

app = Flask(__name__, static_folder='.')
CORS(app)
sock = Sock(app)

# Startup time
START_TIME = datetime.now()

# WebSocket clients
ws_clients = []


def get_uptime():
    """Calculate uptime"""
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
    """Test REAL PostgreSQL connection"""
    try:
        from teambook.teambook_storage_postgresql import PostgreSQLTeambookStorage

        postgres_url = os.environ.get('POSTGRES_URL') or os.environ.get('DATABASE_URL')
        if not postgres_url:
            return {'connected': False, 'error': 'NO POSTGRES_URL SET'}

        start = time.time()
        storage = PostgreSQLTeambookStorage('health-check')
        latency = int((time.time() - start) * 1000)

        notes = storage.read_notes(limit=1)
        note_count = len(notes)

        pool = storage.pool
        pool_used = len(pool._used)
        pool_max = pool.maxconn

        return {
            'connected': True,
            'latency': latency,
            'noteCount': note_count,
            'poolUsed': pool_used,
            'poolMax': pool_max,
            'url': postgres_url.split('@')[1] if '@' in postgres_url else postgres_url
        }

    except Exception as e:
        return {'connected': False, 'error': str(e)[:100]}


def test_redis():
    """Test REAL Redis connection"""
    try:
        import redis

        use_redis = os.environ.get('USE_REDIS', '').lower() == 'true'
        if not use_redis:
            return {'connected': False, 'error': 'USE_REDIS NOT ENABLED'}

        redis_url = os.environ.get('REDIS_URL', 'redis://localhost:6379/0')

        start = time.time()
        r = redis.from_url(redis_url)
        r.ping()
        latency = int((time.time() - start) * 1000)

        info = r.info('memory')
        memory_used = info.get('used_memory_human', 'Unknown')

        return {
            'connected': True,
            'latency': latency,
            'memoryUsed': memory_used,
            'pubsubActive': True,
            'url': redis_url
        }

    except Exception as e:
        return {'connected': False, 'error': str(e)[:100]}


def test_duckdb():
    """Test REAL DuckDB connection"""
    try:
        from teambook.teambook_storage import TeambookStorage

        start = time.time()
        storage = TeambookStorage('health-check')
        latency = int((time.time() - start) * 1000)

        notes = storage.read_notes(limit=1000000)
        note_count = len(notes)

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


def get_real_ai_network():
    """Get REAL AI network from actual teambook data"""
    try:
        current_teambook = os.environ.get('TEAMBOOK_NAME', 'town-hall-qd')
        storage = TeambookStorageAdapter(current_teambook)

        # Get recent notes (last 200 for activity analysis)
        recent_notes = storage.read_notes(limit=200)

        # Track REAL AI activity
        ai_activity = {}

        for note in recent_notes:
            owner = note.get('owner', 'unknown')

            if owner not in ai_activity:
                ai_activity[owner] = {
                    'sent': 0,
                    'lastCommand': None,
                    'lastSeen': note.get('created', ''),
                    'connections': {}
                }

            ai_activity[owner]['sent'] += 1

            if not ai_activity[owner]['lastCommand']:
                summary = note.get('summary', '')
                content = note.get('content', '')
                ai_activity[owner]['lastCommand'] = summary[:50] if summary else content[:50]

            # Track who this AI interacts with
            content_lower = note.get('content', '').lower()
            for other_ai in ai_activity.keys():
                if other_ai != owner and other_ai in content_lower:
                    if other_ai not in ai_activity[owner]['connections']:
                        ai_activity[owner]['connections'][other_ai] = 0
                    ai_activity[owner]['connections'][other_ai] += 1

        # Build AI list with REAL status
        ais = []
        for ai_id, activity in ai_activity.items():
            last_seen = activity['lastSeen']
            if isinstance(last_seen, str):
                try:
                    last_seen_dt = datetime.fromisoformat(last_seen.replace('Z', '+00:00'))
                    age_minutes = (datetime.now() - last_seen_dt.replace(tzinfo=None)).total_seconds() / 60
                except:
                    age_minutes = 999
            else:
                age_minutes = 999

            # REAL status based on activity
            if age_minutes < 10:
                status = 'active'
            elif age_minutes < 60:
                status = 'idle'
            else:
                status = 'not_active'

            ais.append({
                'name': ai_id,
                'status': status,
                'lastCommand': activity['lastCommand'] or 'No recent activity',
                'sent': activity['sent'],
                'connections': activity['connections']
            })

        return ais

    except Exception as e:
        print(f"ERROR getting AI network: {e}")
        return []


@app.route('/')
def index():
    """Serve UI - use v2 version"""
    return send_from_directory('.', 'teambook_health_v2.html')


@app.route('/health_style.css')
def style():
    return send_from_directory('.', 'health_style.css')


@app.route('/health_script_v2.js')
def script():
    return send_from_directory('.', 'health_script_v2.js')


@app.route('/api/health')
def health():
    """Get REAL health status - NO MOCK DATA"""
    postgresql = test_postgresql()
    redis = test_redis()
    duckdb = test_duckdb()

    active_backend = get_storage_backend()

    if postgresql['connected']:
        overall = 'healthy'
    elif redis['connected']:
        overall = 'degraded'
    elif duckdb['connected']:
        overall = 'degraded'
    else:
        overall = 'error'

    # Get REAL AI network data
    active_ais = get_real_ai_network()

    return jsonify({
        'postgresql': postgresql,
        'redis': redis,
        'duckdb': duckdb,
        'activeBackend': active_backend,
        'overall': overall,
        'activeAIs': active_ais,
        'stats': {
            'writesPerSec': 0,  # TODO: Track real writes
            'readsPerSec': 0,   # TODO: Track real reads
            'avgLatency': postgresql.get('latency', duckdb.get('latency', 0)),
            'uptime': get_uptime()
        },
        'timestamp': datetime.now().isoformat()
    })


@app.route('/api/test/<backend>')
def test_backend_endpoint(backend):
    """Test specific backend"""
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


@sock.route('/ws/teambook')
def teambook_websocket(ws):
    """WebSocket endpoint for REAL-TIME updates with resource limits"""
    # Security: Add resource limits to prevent DoS
    MAX_WEBSOCKET_DURATION = 3600  # 1 hour
    MAX_MESSAGES_PER_CONNECTION = 720  # 1 hour @ 5 sec intervals

    start_time = time.time()
    message_count = 0

    print(f"✓ WebSocket client connected: {ws}")
    ws_clients.append(ws)

    try:
        while True:
            # Security: Check duration limit
            if time.time() - start_time > MAX_WEBSOCKET_DURATION:
                print(f"WebSocket connection time limit reached (1 hour)")
                break

            # Security: Check message count limit
            if message_count >= MAX_MESSAGES_PER_CONNECTION:
                print(f"WebSocket message count limit reached")
                break

            # Send real-time AI status every 5 seconds
            time.sleep(5)

            try:
                active_ais = get_real_ai_network()

                message = json.dumps({
                    'type': 'ai_status',
                    'ais': active_ais,
                    'timestamp': datetime.now().isoformat()
                })

                ws.send(message)
                message_count += 1
            except Exception as e:
                print(f"Error sending WebSocket update: {e}")
                break

    except Exception as e:
        print(f"WebSocket error: {e}")
    finally:
        if ws in ws_clients:
            ws_clients.remove(ws)
        duration = time.time() - start_time
        print(f"✗ WebSocket client disconnected (duration: {duration:.1f}s, messages: {message_count})")


def main():
    """Start server"""
    print("=" * 60)
    print("TEAMBOOK HEALTH MONITOR v2 - REAL DATA ONLY")
    print("=" * 60)
    print("")
    print("✓ WebSocket support enabled")
    print("✓ Real backend connections")
    print("✓ Live AI network monitoring")
    print("")
    print("Server: http://localhost:8765")
    print("WebSocket: ws://localhost:8765/ws/teambook")
    print("")
    print("Press Ctrl+C to stop")
    print("=" * 60)

    app.run(host='0.0.0.0', port=8765, debug=False)


if __name__ == '__main__':
    main()
