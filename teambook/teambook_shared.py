"""Compatibility shim for legacy package imports.

This module re-exports the canonical :mod:`teambook_shared` utilities so
packaged integrations that import ``teambook.teambook_shared`` automatically
benefit from the latest security and identity improvements.
"""

from teambook_shared import *  # noqa: F401,F403
import teambook_shared as _canonical_teambook_shared

__all__ = getattr(
    _canonical_teambook_shared,
    '__all__',
    [name for name in globals() if not name.startswith('_')]
)
