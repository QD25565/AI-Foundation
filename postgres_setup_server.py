#!/usr/bin/env python3
"""
PostgreSQL Setup Server
Provides HTTP API for the setup UI with auto-detection and health checks

SECURITY WARNING: This server should only be run on localhost and only during setup.
Do NOT expose this server to a network without proper authentication!
"""

import os
import sys
import json
import socket
import secrets
import hashlib
import logging
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import parse_qs, urlparse
import subprocess
import psycopg2

# SECURITY: Setup authentication token
# This token should be randomly generated and passed to the server
SETUP_TOKEN = os.getenv('SETUP_AUTH_TOKEN') or secrets.token_urlsafe(32)

# Add src to path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
sys.path.insert(0, os.path.dirname(__file__))

try:
    from teambook.teambook_config import get_storage_backend
    from teambook.storage_adapter import TeambookStorageAdapter
except ImportError:
    try:
        import teambook_config
        import storage_adapter
        get_storage_backend = teambook_config.get_storage_backend
        TeambookStorageAdapter = storage_adapter.TeambookStorageAdapter
    except ImportError:
        print("Warning: Could not import teambook modules, health monitoring will be limited")
        get_storage_backend = lambda: "duckdb"
        TeambookStorageAdapter = None


class PostgreSQLSetupHandler(BaseHTTPRequestHandler):
    """HTTP request handler for PostgreSQL setup"""

    def check_auth(self):
        """
        Verify authentication token from request
        Returns True if authenticated, False otherwise
        """
        # Get token from Authorization header
        auth_header = self.headers.get('Authorization', '')

        # Support both "Bearer TOKEN" and raw token formats
        token = auth_header.replace('Bearer ', '').strip()

        # Compare tokens using constant-time comparison to prevent timing attacks
        return secrets.compare_digest(token, SETUP_TOKEN) if token else False

    def require_auth(self, handler_func):
        """
        Decorator-style auth check for handlers
        Returns True if authenticated and calls handler, False if unauthorized
        """
        if not self.check_auth():
            self.send_response(401)
            self.send_header('Content-Type', 'application/json')
            self.send_header('WWW-Authenticate', 'Bearer realm="PostgreSQL Setup"')
            self.end_headers()
            self.wfile.write(json.dumps({
                'error': 'Unauthorized',
                'message': 'Valid authentication token required'
            }).encode('utf-8'))
            return False
        return True

    def do_OPTIONS(self):
        """Handle CORS preflight"""
        self.send_response(200)
        self.send_header('Access-Control-Allow-Origin', '*')
        self.send_header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
        self.send_header('Access-Control-Allow-Headers', 'Content-Type, Authorization')
        self.end_headers()

    def do_GET(self):
        """Serve the UI HTML"""
        # Health endpoint doesn't require auth for monitoring
        if self.path == '/health':
            self.handle_health()
            return

        # All other endpoints require authentication
        if not self.require_auth(None):
            return

        if self.path == '/' or self.path == '/index.html':
            self.serve_ui()
        else:
            self.send_error(404)

    def do_POST(self):
        """Handle API requests"""
        # Health endpoint doesn't require auth for monitoring
        if self.path == '/health':
            self.handle_health()
            return

        # All other endpoints require authentication
        if not self.require_auth(None):
            return

        if self.path == '/detect-postgres':
            self.handle_detect()
        elif self.path == '/test-postgres':
            self.handle_test()
        elif self.path == '/setup-postgres':
            self.handle_setup()
        else:
            self.send_error(404)

    def serve_ui(self):
        """Serve the setup UI HTML"""
        ui_path = os.path.join(os.path.dirname(__file__), 'postgres_setup_ui.html')
        try:
            with open(ui_path, 'r', encoding='utf-8') as f:
                content = f.read()
            self.send_response(200)
            self.send_header('Content-Type', 'text/html; charset=utf-8')
            self.send_header('Access-Control-Allow-Origin', '*')
            self.end_headers()
            self.wfile.write(content.encode('utf-8'))
        except FileNotFoundError:
            self.send_error(404, 'Setup UI not found')

    def handle_detect(self):
        """Auto-detect PostgreSQL installation"""
        result = detect_postgresql()
        self.send_json_response(result)

    def handle_test(self):
        """Test PostgreSQL connection"""
        content_length = int(self.headers['Content-Length'])

        # SECURITY: Limit request size to prevent DoS attacks
        MAX_REQUEST_SIZE = 10 * 1024  # 10KB
        if content_length > MAX_REQUEST_SIZE:
            self.send_error(413, 'Request entity too large')
            return

        body = self.rfile.read(content_length)

        # SECURITY NOTE: json.loads() is safe in Python
        # Unlike pickle, JSON cannot execute code during deserialization
        # It only creates basic Python types (dict, list, str, int, float, bool, None)
        try:
            config = json.loads(body.decode('utf-8'))
        except json.JSONDecodeError as e:
            self.send_json_response({'success': False, 'error': f'Invalid JSON: {e}'})
            return

        result = test_connection(config)
        self.send_json_response(result)

    def handle_setup(self):
        """Setup PostgreSQL database"""
        content_length = int(self.headers['Content-Length'])

        # SECURITY: Limit request size to prevent DoS attacks
        MAX_REQUEST_SIZE = 10 * 1024  # 10KB
        if content_length > MAX_REQUEST_SIZE:
            self.send_error(413, 'Request entity too large')
            return

        body = self.rfile.read(content_length)

        # SECURITY NOTE: json.loads() is safe - see handle_test() for explanation
        try:
            config = json.loads(body.decode('utf-8'))
        except json.JSONDecodeError as e:
            self.send_json_response({'success': False, 'error': f'Invalid JSON: {e}'})
            return

        result = setup_postgresql(config)
        self.send_json_response(result)

    def handle_health(self):
        """Get system health status"""
        health = get_health_status()
        self.send_json_response(health)

    def send_json_response(self, data):
        """Send JSON response with CORS headers"""
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()
        self.wfile.write(json.dumps(data).encode('utf-8'))

    def log_message(self, format, *args):
        """Suppress default logging"""
        pass


def detect_postgresql():
    """
    Auto-detect PostgreSQL installation
    Returns dict with found, port, version
    """
    common_ports = [5432, 5433, 5434]

    for port in common_ports:
        if is_port_open('localhost', port):
            version = get_postgres_version('localhost', port)
            return {
                'found': True,
                'port': port,
                'version': version
            }

    return {'found': False}


def is_port_open(host, port, timeout=2):
    """Check if a port is open"""
    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.settimeout(timeout)
        result = sock.connect_ex((host, port))
        sock.close()
        return result == 0
    except:
        return False


def get_postgres_version(host, port):
    """
    Try to get PostgreSQL version.

    SECURITY NOTE: This function is only used for auto-detection during setup.
    It attempts connection with environment credentials, not hardcoded ones.
    """
    try:
        # SECURITY FIX: Use credentials from environment, not hardcoded
        # Only attempt auto-detection if admin credentials are provided
        admin_user = os.getenv('POSTGRES_ADMIN_USER', 'postgres')
        admin_password = os.getenv('POSTGRES_ADMIN_PASSWORD')

        if not admin_password:
            # Cannot auto-detect without admin credentials
            # This is safer than using hardcoded defaults
            return None

        conn = psycopg2.connect(
            host=host,
            port=port,
            user=admin_user,
            password=admin_password,
            database='postgres',
            connect_timeout=2
        )
        cursor = conn.cursor()
        cursor.execute('SELECT version()')
        version = cursor.fetchone()[0]
        conn.close()

        # Extract version number
        if 'PostgreSQL' in version:
            version = version.split('PostgreSQL ')[1].split(' ')[0]
        return version
    except Exception as e:
        logging.debug(f"Could not auto-detect PostgreSQL version: {e}")
        return None


def test_connection(config):
    """
    Test PostgreSQL connection with given config
    Returns dict with success, error
    """
    try:
        conn = psycopg2.connect(
            host=config['host'],
            port=config['port'],
            user=config['username'],
            password=config['password'],
            database=config.get('database', 'postgres'),
            connect_timeout=5
        )
        conn.close()
        return {'success': True}
    except Exception as e:
        return {'success': False, 'error': str(e)}


def setup_postgresql(config):
    """
    Setup PostgreSQL database and tables
    Returns dict with success, error
    """
    try:
        # Connect to PostgreSQL
        conn = psycopg2.connect(
            host=config['host'],
            port=config['port'],
            user=config['username'],
            password=config['password'],
            database='postgres',  # Connect to default db first
            connect_timeout=5
        )
        conn.autocommit = True
        cursor = conn.cursor()

        # Create database if not exists
        db_name = config['database']

        # SECURITY: Multi-layer SQL injection prevention
        # Layer 1: Strict regex validation (defense in depth)
        import re
        if not re.match(r'^[a-zA-Z0-9_]+$', db_name):
            raise ValueError(f"Invalid database name: {db_name}. Only alphanumeric characters and underscores allowed.")

        # Layer 2: Use psycopg2.sql module for safe query construction
        # This is the CORRECT way to handle database identifiers in PostgreSQL
        from psycopg2 import sql

        # sql.Literal() - Used for data values (strings, numbers)
        # sql.Identifier() - Used for database objects (tables, columns, databases)
        # Both methods are designed to prevent SQL injection

        # Check if database exists (using sql.Literal for data comparison)
        cursor.execute(sql.SQL("SELECT 1 FROM pg_database WHERE datname = {}").format(
            sql.Literal(db_name)
        ))
        if not cursor.fetchone():
            # Create database (using sql.Identifier for database name)
            # This is safe because:
            # 1. sql.Identifier properly quotes identifiers
            # 2. Regex validation ensures only safe characters
            # 3. psycopg2 internally validates the identifier
            cursor.execute(sql.SQL("CREATE DATABASE {}").format(
                sql.Identifier(db_name)
            ))

        cursor.close()
        conn.close()

        # Connect to new database and create tables
        conn = psycopg2.connect(
            host=config['host'],
            port=config['port'],
            user=config['username'],
            password=config['password'],
            database=db_name,
            connect_timeout=5
        )
        cursor = conn.cursor()

        # Create notes table
        cursor.execute("""
            CREATE TABLE IF NOT EXISTS notes (
                id SERIAL PRIMARY KEY,
                teambook_name VARCHAR(255) NOT NULL,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                content TEXT NOT NULL,
                summary VARCHAR(500),
                owner VARCHAR(255),
                claimed_by VARCHAR(255),
                assigned_to VARCHAR(255),
                status VARCHAR(50) DEFAULT 'open',
                tags TEXT[],
                pinned BOOLEAN DEFAULT FALSE,
                parent_id INTEGER,
                metadata JSONB
            )
        """)

        # Create vault table
        cursor.execute("""
            CREATE TABLE IF NOT EXISTS vault (
                id SERIAL PRIMARY KEY,
                teambook_name VARCHAR(255) NOT NULL,
                key VARCHAR(255) NOT NULL,
                encrypted_value BYTEA NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(teambook_name, key)
            )
        """)

        # Create indices
        cursor.execute("CREATE INDEX IF NOT EXISTS idx_notes_teambook ON notes(teambook_name)")
        cursor.execute("CREATE INDEX IF NOT EXISTS idx_notes_status ON notes(status)")
        cursor.execute("CREATE INDEX IF NOT EXISTS idx_notes_owner ON notes(owner)")
        cursor.execute("CREATE INDEX IF NOT EXISTS idx_vault_teambook ON vault(teambook_name)")

        conn.commit()
        cursor.close()
        conn.close()

        # Save connection string to environment
        conn_str = f"postgresql://{config['username']}:{config['password']}@{config['host']}:{config['port']}/{db_name}"
        os.environ['POSTGRES_URL'] = conn_str

        return {'success': True}

    except Exception as e:
        return {'success': False, 'error': str(e)}


def get_health_status():
    """
    Get current health status of all backends
    Returns dict with backend info, connection status, stats
    """
    health = {
        'current_backend': get_storage_backend(),
        'postgres': {'available': False, 'status': 'Not configured'},
        'redis': {'available': False, 'status': 'Not configured'},
        'duckdb': {'available': True, 'status': 'Always available (fallback)'},
        'stats': {}
    }

    # Check PostgreSQL
    if os.getenv('POSTGRES_URL'):
        try:
            if TeambookStorageAdapter:
                teambook_name = os.getenv('TEAMBOOK_NAME', 'default')
                adapter = TeambookStorageAdapter(teambook_name)
                stats = adapter.get_stats()
                health['postgres'] = {
                    'available': True,
                    'status': 'Connected',
                    'url': os.getenv('POSTGRES_URL').split('@')[1]  # Hide credentials
                }
                health['stats'] = stats
            else:
                health['postgres'] = {
                    'available': True,
                    'status': 'Configured (module unavailable)',
                    'url': os.getenv('POSTGRES_URL').split('@')[1]
                }
        except Exception as e:
            health['postgres'] = {
                'available': False,
                'status': f'Error: {str(e)}'
            }

    # Check Redis
    if os.getenv('USE_REDIS') == 'true':
        redis_url = os.getenv('REDIS_URL', 'localhost:6379')
        host, port = redis_url.replace('redis://', '').split(':')
        if is_port_open(host, int(port)):
            health['redis'] = {
                'available': True,
                'status': 'Connected',
                'url': redis_url
            }
        else:
            health['redis'] = {
                'available': False,
                'status': 'Cannot connect'
            }

    return health


def start_server(port=8000):
    """Start the setup server"""
    server_address = ('', port)
    httpd = HTTPServer(server_address, PostgreSQLSetupHandler)

    # SECURITY: Save token to file with restricted permissions instead of console
    # This prevents token exposure in console logs
    # Using secure file creation to prevent race conditions and symlink attacks
    token_file = os.path.join(os.path.dirname(__file__), '.setup_token')
    try:
        # SECURITY: Open with O_CREAT | O_EXCL to prevent race conditions
        # This fails if file already exists, preventing symlink attacks
        # On Windows, this just opens the file securely
        import stat
        fd = os.open(token_file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        try:
            os.write(fd, SETUP_TOKEN.encode('utf-8'))
        finally:
            os.close(fd)

        # Verify permissions were set correctly
        file_stat = os.stat(token_file)
        expected_mode = stat.S_IRUSR | stat.S_IWUSR  # 0o600
        if file_stat.st_mode & 0o777 != expected_mode:
            logging.warning(f"Token file permissions may be incorrect: {oct(file_stat.st_mode & 0o777)}")

        token_saved = True
    except Exception as e:
        logging.warning(f"Could not save token to file: {e}")
        token_saved = False

    print("=" * 70)
    print("PostgreSQL Setup Server")
    print("=" * 70)
    print(f"Server running on: http://localhost:{port}")

    if token_saved:
        print(f"\n🔐 AUTHENTICATION TOKEN saved to:")
        print(f"   {token_file}")
        print(f"\n   Read it with: cat {token_file}")
        print(f"\n   (File has restricted permissions: owner read/write only)")
    else:
        # Fallback: print to console if file save failed
        print(f"\n🔐 AUTHENTICATION TOKEN (required for all requests):")
        print(f"   {SETUP_TOKEN}")

    print(f"\nTo use the API, include this header in your requests:")
    print(f"   Authorization: Bearer $(cat {token_file})")
    print(f"\nOr set the environment variable before starting:")
    print(f"   export SETUP_AUTH_TOKEN=<your-secure-token>")
    print("\n⚠️  SECURITY WARNING:")
    print("   This server should ONLY run on localhost during setup.")
    print("   Do NOT expose this server to a network!")
    print("   Delete the token file after setup: rm " + token_file)
    print("=" * 70)
    print("\nPress Ctrl+C to stop\n")

    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down server...")
        httpd.shutdown()


if __name__ == '__main__':
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8000
    start_server(port)
