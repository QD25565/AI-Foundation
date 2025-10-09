#!/usr/bin/env python3
"""
PostgreSQL Backend Diagnostics
===============================
Debug and diagnostic tools for PostgreSQL integration.
Provides connection testing, schema validation, and performance metrics.
"""

import os
import sys
import time
import logging
from pathlib import Path
from typing import Dict, List, Optional

sys.path.insert(0, str(Path(__file__).parent))

from teambook_config import get_storage_backend, use_postgresql
from storage_adapter import TeambookStorageAdapter

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


def check_postgres_available() -> Dict:
    """Check if PostgreSQL is available and configured"""
    result = {
        'psycopg2_installed': False,
        'postgres_url_set': False,
        'connection_successful': False,
        'backend_detected': None,
        'errors': []
    }

    # Check psycopg2 installation
    try:
        import psycopg2
        result['psycopg2_installed'] = True
        result['psycopg2_version'] = psycopg2.__version__
    except ImportError as e:
        result['errors'].append(f"psycopg2 not installed: {e}")

    # Check environment variables
    postgres_url = os.getenv('POSTGRES_URL') or os.getenv('DATABASE_URL')
    if postgres_url:
        result['postgres_url_set'] = True
        # Redact password for security
        safe_url = postgres_url.split('@')[0].split(':')[0] + ':***@' + postgres_url.split('@')[1] if '@' in postgres_url else '***'
        result['postgres_url'] = safe_url
    else:
        result['errors'].append("No POSTGRES_URL or DATABASE_URL set")

    # Check backend detection
    try:
        backend = get_storage_backend()
        result['backend_detected'] = backend
    except Exception as e:
        result['errors'].append(f"Backend detection failed: {e}")

    # Test connection
    if result['psycopg2_installed'] and result['postgres_url_set']:
        try:
            import psycopg2
            conn = psycopg2.connect(postgres_url)
            conn.close()
            result['connection_successful'] = True
        except Exception as e:
            result['errors'].append(f"Connection test failed: {e}")

    return result


def validate_schema(teambook_name: str = "diagnostic-test") -> Dict:
    """Validate PostgreSQL schema is correctly created"""
    result = {
        'schema_valid': False,
        'tables_found': [],
        'tables_missing': [],
        'indices_found': [],
        'errors': []
    }

    expected_tables = [
        'notes', 'edges', 'entities', 'entity_notes',
        'sessions', 'vault', 'stats', 'teambooks',
        'evolution_outputs'
    ]

    try:
        from teambook_storage_postgresql import get_pg_conn

        with get_pg_conn() as conn:
            with conn.cursor() as cur:
                # Check tables
                cur.execute("""
                    SELECT tablename FROM pg_tables
                    WHERE schemaname = 'public'
                """)
                tables = [row[0] for row in cur.fetchall()]
                result['tables_found'] = tables

                for table in expected_tables:
                    if table not in tables:
                        result['tables_missing'].append(table)

                # Check indices
                cur.execute("""
                    SELECT indexname FROM pg_indexes
                    WHERE schemaname = 'public'
                """)
                indices = [row[0] for row in cur.fetchall()]
                result['indices_found'] = indices

                result['schema_valid'] = len(result['tables_missing']) == 0

    except Exception as e:
        result['errors'].append(f"Schema validation failed: {e}")

    return result


def benchmark_operations(teambook_name: str = "benchmark-test", iterations: int = 100) -> Dict:
    """Benchmark PostgreSQL backend performance"""
    result = {
        'iterations': iterations,
        'write_avg_ms': 0,
        'read_avg_ms': 0,
        'update_avg_ms': 0,
        'delete_avg_ms': 0,
        'total_time_ms': 0,
        'errors': []
    }

    try:
        storage = TeambookStorageAdapter(teambook_name)

        write_times = []
        read_times = []
        update_times = []
        delete_times = []

        start_total = time.time()

        for i in range(iterations):
            # Write
            t1 = time.time()
            note_id = storage.write_note(content=f"Benchmark note {i}", summary=f"Test {i}")
            write_times.append((time.time() - t1) * 1000)

            # Read
            t2 = time.time()
            storage.get_note(note_id)
            read_times.append((time.time() - t2) * 1000)

            # Update
            t3 = time.time()
            storage.update_note(note_id, pinned=True)
            update_times.append((time.time() - t3) * 1000)

            # Delete
            t4 = time.time()
            storage.delete_note(note_id)
            delete_times.append((time.time() - t4) * 1000)

        total_time = (time.time() - start_total) * 1000

        result['write_avg_ms'] = sum(write_times) / len(write_times)
        result['read_avg_ms'] = sum(read_times) / len(read_times)
        result['update_avg_ms'] = sum(update_times) / len(update_times)
        result['delete_avg_ms'] = sum(delete_times) / len(delete_times)
        result['total_time_ms'] = total_time
        result['ops_per_second'] = (iterations * 4) / (total_time / 1000)

    except Exception as e:
        result['errors'].append(f"Benchmark failed: {e}")

    return result


def test_connection_pool(min_conn: int = 2, max_conn: int = 10) -> Dict:
    """Test connection pool behavior"""
    result = {
        'pool_initialized': False,
        'min_connections': min_conn,
        'max_connections': max_conn,
        'concurrent_test_passed': False,
        'errors': []
    }

    try:
        from teambook_storage_postgresql import init_connection_pool, _connection_pool

        # Initialize pool
        success = init_connection_pool(min_conn=min_conn, max_conn=max_conn)
        result['pool_initialized'] = success

        if success:
            # Test concurrent access
            import concurrent.futures
            storage = TeambookStorageAdapter("pool-test")

            def concurrent_write(i):
                return storage.write_note(content=f"Concurrent {i}")

            with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
                futures = [executor.submit(concurrent_write, i) for i in range(20)]
                results = [f.result() for f in futures]

            # All should succeed
            if len(results) == 20 and all(r > 0 for r in results):
                result['concurrent_test_passed'] = True

            # Cleanup
            for note_id in results:
                storage.delete_note(note_id)

    except Exception as e:
        result['errors'].append(f"Pool test failed: {e}")

    return result


def run_full_diagnostics() -> Dict:
    """Run complete diagnostic suite"""
    print("=" * 60)
    print("PostgreSQL Backend Diagnostics")
    print("=" * 60)
    print()

    diagnostics = {}

    # Connection check
    print("1. Checking PostgreSQL availability...")
    conn_check = check_postgres_available()
    diagnostics['connection'] = conn_check

    if conn_check['connection_successful']:
        print("   ✓ PostgreSQL connection successful")
        print(f"   ✓ Backend detected: {conn_check['backend_detected']}")
    else:
        print("   ✗ PostgreSQL connection failed")
        for error in conn_check['errors']:
            print(f"     - {error}")
        return diagnostics

    # Schema validation
    print("\n2. Validating schema...")
    schema_check = validate_schema()
    diagnostics['schema'] = schema_check

    if schema_check['schema_valid']:
        print(f"   ✓ Schema valid ({len(schema_check['tables_found'])} tables)")
        print(f"   ✓ Indices created ({len(schema_check['indices_found'])} indices)")
    else:
        print("   ✗ Schema incomplete")
        print(f"     Missing tables: {schema_check['tables_missing']}")

    # Connection pool
    print("\n3. Testing connection pool...")
    pool_test = test_connection_pool()
    diagnostics['connection_pool'] = pool_test

    if pool_test['pool_initialized']:
        print("   ✓ Connection pool initialized")
        if pool_test['concurrent_test_passed']:
            print("   ✓ Concurrent access test passed")
    else:
        print("   ✗ Connection pool test failed")

    # Performance benchmark
    print("\n4. Running performance benchmark (100 operations)...")
    benchmark = benchmark_operations(iterations=100)
    diagnostics['benchmark'] = benchmark

    if not benchmark['errors']:
        print(f"   Write:  {benchmark['write_avg_ms']:.2f}ms avg")
        print(f"   Read:   {benchmark['read_avg_ms']:.2f}ms avg")
        print(f"   Update: {benchmark['update_avg_ms']:.2f}ms avg")
        print(f"   Delete: {benchmark['delete_avg_ms']:.2f}ms avg")
        print(f"   Total:  {benchmark['ops_per_second']:.0f} ops/sec")
    else:
        print("   ✗ Benchmark failed")

    print()
    print("=" * 60)
    print("Diagnostics complete")
    print("=" * 60)

    return diagnostics


if __name__ == "__main__":
    import json
    results = run_full_diagnostics()

    # Save results to file
    output_file = Path(__file__).parent.parent.parent / "postgres_diagnostics_report.json"
    with open(output_file, 'w') as f:
        json.dump(results, f, indent=2, default=str)

    print(f"\nFull report saved to: {output_file}")
