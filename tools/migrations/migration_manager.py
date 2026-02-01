#!/usr/bin/env python3
"""
MIGRATION MANAGER - World-Class Database Schema Management
===========================================================

Enterprise-grade database migration system for AI-Foundation tools.
Inspired by Alembic, Flyway, and Rails migrations.

Features:
- Version-controlled schema changes
- Up/down migrations (reversible)
- Migration history tracking
- Status reporting
- PostgreSQL support

Usage:
    # Show current status
    python -m tools.migrations.migration_manager status

    # Apply all pending migrations
    python -m tools.migrations.migration_manager up

    # Apply specific migration
    python -m tools.migrations.migration_manager up --version 001

    # Rollback last migration
    python -m tools.migrations.migration_manager down

    # Rollback to specific version
    python -m tools.migrations.migration_manager down --version 001

    # Create new migration from template
    python -m tools.migrations.migration_manager create --name "add_user_table"

Architecture:
1. Migrations stored in tools/migrations/versions/
2. Each migration has UP and DOWN sections
3. Applied migrations tracked in schema_migrations table
4. Atomic operations (transaction per migration)
5. Validation before applying

Migration File Format:
    -- Migration: 001_projects_schema
    -- Description: Create projects and project_features tables
    -- UP
    CREATE TABLE projects (...);
    CREATE TABLE project_features (...);

    -- DOWN
    DROP TABLE project_features;
    DROP TABLE projects;
"""

import os
import sys
import re
import argparse
from pathlib import Path
from typing import List, Dict, Optional, Tuple
from datetime import datetime
import logging

# Add tools to path
sys.path.insert(0, str(Path(__file__).parent.parent.parent))

from tools.teambook.teambook_utils import get_db_conn

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s [%(levelname)s] %(message)s'
)
log = logging.getLogger(__name__)


class Migration:
    """Represents a single database migration."""

    def __init__(self, version: str, name: str, file_path: Path):
        """
        Initialize migration.

        Args:
            version: Migration version (e.g., "001")
            name: Migration name (e.g., "projects_schema")
            file_path: Path to migration SQL file
        """
        self.version = version
        self.name = name
        self.file_path = file_path
        self.up_sql: Optional[str] = None
        self.down_sql: Optional[str] = None
        self.description: Optional[str] = None

    def parse(self) -> None:
        """Parse migration file to extract UP and DOWN SQL."""
        with open(self.file_path, 'r', encoding='utf-8') as f:
            content = f.read()

        # Extract description
        desc_match = re.search(r'--\s*Description:\s*(.+)', content)
        if desc_match:
            self.description = desc_match.group(1).strip()

        # Split into UP and DOWN sections
        parts = re.split(r'--\s*DOWN\s*\n', content, maxsplit=1, flags=re.IGNORECASE)

        if len(parts) == 2:
            up_section = parts[0]
            down_section = parts[1]

            # Extract UP SQL (after "-- UP" marker)
            up_match = re.search(r'--\s*UP\s*\n(.*)', up_section, re.DOTALL | re.IGNORECASE)
            if up_match:
                self.up_sql = up_match.group(1).strip()

            # DOWN SQL is everything after "-- DOWN"
            self.down_sql = down_section.strip()
        else:
            # No DOWN section - only UP
            up_match = re.search(r'--\s*UP\s*\n(.*)', content, re.DOTALL | re.IGNORECASE)
            if up_match:
                self.up_sql = up_match.group(1).strip()
            else:
                # No markers - assume entire file is UP
                self.up_sql = content.strip()

        if not self.up_sql:
            raise ValueError(f"Migration {self.version} has no UP SQL")

    def __repr__(self) -> str:
        return f"Migration({self.version}_{self.name})"


class MigrationManager:
    """Manages database migrations."""

    def __init__(self, migrations_dir: Optional[Path] = None):
        """
        Initialize migration manager.

        Args:
            migrations_dir: Path to migrations directory (default: tools/migrations/versions/)
        """
        if migrations_dir is None:
            migrations_dir = Path(__file__).parent / 'versions'

        self.migrations_dir = migrations_dir
        self.migrations_dir.mkdir(parents=True, exist_ok=True)

    def _ensure_migrations_table(self) -> None:
        """Ensure schema_migrations table exists."""
        with get_db_conn() as conn:
            conn.execute("""
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version VARCHAR(10) PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT,
                    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    applied_by TEXT,
                    checksum TEXT
                )
            """)
            conn.commit()

    def get_all_migrations(self) -> List[Migration]:
        """
        Get all migrations from filesystem.

        Returns:
            List of Migration objects, sorted by version
        """
        migrations = []

        for file_path in sorted(self.migrations_dir.glob('*.sql')):
            # Parse filename: 001_projects_schema.sql
            match = re.match(r'(\d+)_(.+)\.sql', file_path.name)
            if match:
                version, name = match.groups()
                migration = Migration(version, name, file_path)
                try:
                    migration.parse()
                    migrations.append(migration)
                except Exception as e:
                    log.warning(f"Failed to parse {file_path.name}: {e}")

        return sorted(migrations, key=lambda m: m.version)

    def get_applied_migrations(self) -> List[str]:
        """
        Get list of applied migration versions.

        Returns:
            List of version strings (e.g., ["001", "002"])
        """
        self._ensure_migrations_table()

        with get_db_conn() as conn:
            results = conn.execute(
                "SELECT version FROM schema_migrations ORDER BY version"
            ).fetchall()

        return [row['version'] for row in results]

    def get_pending_migrations(self) -> List[Migration]:
        """
        Get migrations that haven't been applied yet.

        Returns:
            List of pending migrations
        """
        all_migrations = self.get_all_migrations()
        applied = set(self.get_applied_migrations())

        return [m for m in all_migrations if m.version not in applied]

    def get_status(self) -> Dict[str, any]:
        """
        Get current migration status.

        Returns:
            Dictionary with status information
        """
        all_migrations = self.get_all_migrations()
        applied = set(self.get_applied_migrations())

        status = {
            'total': len(all_migrations),
            'applied': len(applied),
            'pending': len(all_migrations) - len(applied),
            'migrations': []
        }

        for migration in all_migrations:
            status['migrations'].append({
                'version': migration.version,
                'name': migration.name,
                'description': migration.description,
                'applied': migration.version in applied
            })

        return status

    def apply_migration(self, migration: Migration) -> None:
        """
        Apply a single migration (UP).

        Args:
            migration: Migration to apply

        Raises:
            Exception: If migration fails
        """
        log.info(f"Applying migration {migration.version}_{migration.name}...")

        try:
            with get_db_conn() as conn:
                # Execute UP SQL
                conn.execute(migration.up_sql)

                # Record in schema_migrations
                conn.execute("""
                    INSERT INTO schema_migrations (version, name, description, applied_by)
                    VALUES (?, ?, ?, ?)
                """, [
                    migration.version,
                    migration.name,
                    migration.description,
                    os.getenv('AI_ID', 'unknown')
                ])

                conn.commit()

            log.info(f"Successfully applied migration {migration.version}")

        except Exception as e:
            log.error(f"Failed to apply migration {migration.version}: {e}")
            raise

    def rollback_migration(self, migration: Migration) -> None:
        """
        Rollback a single migration (DOWN).

        Args:
            migration: Migration to rollback

        Raises:
            Exception: If rollback fails or no DOWN SQL
        """
        if not migration.down_sql:
            raise ValueError(f"Migration {migration.version} has no DOWN SQL - cannot rollback")

        log.info(f"Rolling back migration {migration.version}_{migration.name}...")

        try:
            with get_db_conn() as conn:
                # Execute DOWN SQL
                conn.execute(migration.down_sql)

                # Remove from schema_migrations
                conn.execute(
                    "DELETE FROM schema_migrations WHERE version = ?",
                    [migration.version]
                )

                conn.commit()

            log.info(f"Successfully rolled back migration {migration.version}")

        except Exception as e:
            log.error(f"Failed to rollback migration {migration.version}: {e}")
            raise

    def migrate_up(self, target_version: Optional[str] = None) -> int:
        """
        Apply pending migrations.

        Args:
            target_version: Apply migrations up to this version (default: all)

        Returns:
            Number of migrations applied
        """
        pending = self.get_pending_migrations()

        if not pending:
            log.info("No pending migrations")
            return 0

        # Filter by target version
        if target_version:
            pending = [m for m in pending if m.version <= target_version]

        if not pending:
            log.info(f"No migrations to apply up to version {target_version}")
            return 0

        log.info(f"Applying {len(pending)} migration(s)...")

        for migration in pending:
            self.apply_migration(migration)

        return len(pending)

    def migrate_down(self, target_version: Optional[str] = None) -> int:
        """
        Rollback migrations.

        Args:
            target_version: Rollback down to this version (default: rollback last)

        Returns:
            Number of migrations rolled back
        """
        all_migrations = self.get_all_migrations()
        applied = self.get_applied_migrations()

        if not applied:
            log.info("No migrations to rollback")
            return 0

        # Build list of migrations to rollback (in reverse order)
        to_rollback = []

        if target_version:
            # Rollback everything after target_version
            for version in reversed(applied):
                if version > target_version:
                    # Find migration object
                    migration = next((m for m in all_migrations if m.version == version), None)
                    if migration:
                        to_rollback.append(migration)
        else:
            # Rollback only the last migration
            last_version = applied[-1]
            migration = next((m for m in all_migrations if m.version == last_version), None)
            if migration:
                to_rollback = [migration]

        if not to_rollback:
            log.info("No migrations to rollback")
            return 0

        log.info(f"Rolling back {len(to_rollback)} migration(s)...")

        for migration in to_rollback:
            self.rollback_migration(migration)

        return len(to_rollback)

    def create_migration(self, name: str) -> Path:
        """
        Create new migration file from template.

        Args:
            name: Migration name (e.g., "add_user_table")

        Returns:
            Path to created migration file
        """
        # Find next version number
        existing = self.get_all_migrations()
        if existing:
            last_version = int(existing[-1].version)
            next_version = f"{last_version + 1:03d}"
        else:
            next_version = "001"

        # Create filename
        filename = f"{next_version}_{name}.sql"
        file_path = self.migrations_dir / filename

        # Write template
        template = f"""-- Migration: {next_version}_{name}
-- Description: TODO - Add description here
-- Created: {datetime.now().isoformat()}
-- Author: {os.getenv('AI_ID', 'unknown')}

-- UP
-- TODO: Add your UP migration SQL here
-- Example:
-- CREATE TABLE example (
--     id SERIAL PRIMARY KEY,
--     name TEXT NOT NULL
-- );

-- DOWN
-- TODO: Add your DOWN migration SQL here
-- Example:
-- DROP TABLE example;
"""

        with open(file_path, 'w', encoding='utf-8') as f:
            f.write(template)

        log.info(f"Created migration: {filename}")
        return file_path


def main():
    """CLI entry point."""
    parser = argparse.ArgumentParser(
        description='AI-Foundation Database Migration Manager'
    )

    subparsers = parser.add_subparsers(dest='command', help='Command to execute')

    # Status command
    subparsers.add_parser('status', help='Show migration status')

    # Up command
    up_parser = subparsers.add_parser('up', help='Apply migrations')
    up_parser.add_argument('--version', help='Apply up to this version')

    # Down command
    down_parser = subparsers.add_parser('down', help='Rollback migrations')
    down_parser.add_argument('--version', help='Rollback down to this version')

    # Create command
    create_parser = subparsers.add_parser('create', help='Create new migration')
    create_parser.add_argument('--name', required=True, help='Migration name')

    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        return

    manager = MigrationManager()

    try:
        if args.command == 'status':
            status = manager.get_status()
            print(f"\nMigration Status:")
            print(f"  Total: {status['total']}")
            print(f"  Applied: {status['applied']}")
            print(f"  Pending: {status['pending']}")
            print(f"\nMigrations:")
            for m in status['migrations']:
                status_icon = '[x]' if m['applied'] else '[ ]'
                desc = m['description'] or 'No description'
                print(f"  {status_icon} {m['version']}_{m['name']} - {desc}")

        elif args.command == 'up':
            count = manager.migrate_up(args.version)
            print(f"\nApplied {count} migration(s)")

        elif args.command == 'down':
            count = manager.migrate_down(args.version)
            print(f"\nRolled back {count} migration(s)")

        elif args.command == 'create':
            file_path = manager.create_migration(args.name)
            print(f"\nCreated: {file_path}")
            print(f"Next steps:")
            print(f"  1. Edit the file and add your SQL")
            print(f"  2. Test the migration: python -m tools.migrations.migration_manager up")

    except Exception as e:
        log.error(f"Command failed: {e}")
        sys.exit(1)


if __name__ == '__main__':
    main()
