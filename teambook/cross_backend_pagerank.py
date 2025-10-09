"""
Cross-backend PageRank calculation.

Works with PostgreSQL, Redis, and DuckDB backends by:
1. Fetching edges via storage adapter
2. Calculating PageRank in Python (backend-agnostic)
3. Updating scores via storage adapter
"""

from typing import Dict, List
import logging

logger = logging.getLogger(__name__)


def calculate_pagerank(adapter, damping: float = 0.85, iterations: int = 20) -> Dict[int, float]:
    """
    Calculate PageRank for all notes in the teambook.

    Args:
        adapter: Storage adapter instance
        damping: Damping factor (default: 0.85)
        iterations: Number of iterations (default: 20)

    Returns:
        Dictionary mapping note_id to pagerank score
    """
    # Get all notes
    try:
        notes = adapter.read_notes(limit=10000, mode='recent')  # Get all notes
    except Exception as e:
        logger.error(f"Failed to read notes for PageRank: {e}")
        return {}

    if not notes:
        return {}

    # Build note ID list
    note_ids = [n['id'] for n in notes]
    num_notes = len(note_ids)

    if num_notes == 0:
        return {}

    # Build edge graph
    outlinks = {}  # note_id -> [list of outgoing note_ids]
    backlinks = {}  # note_id -> [list of incoming note_ids]

    for note_id in note_ids:
        try:
            edges = adapter.get_edges(note_id, reverse=False)
            outlinks[note_id] = [e['to_id'] for e in edges if e['to_id'] in note_ids]

            reverse_edges = adapter.get_edges(note_id, reverse=True)
            backlinks[note_id] = [e['from_id'] for e in reverse_edges if e['from_id'] in note_ids]
        except Exception as e:
            logger.debug(f"Failed to get edges for note {note_id}: {e}")
            outlinks[note_id] = []
            backlinks[note_id] = []

    # Initialize PageRank scores
    pagerank = {note_id: 1.0 / num_notes for note_id in note_ids}

    # PageRank iteration
    for iteration in range(iterations):
        new_pagerank = {}

        for note_id in note_ids:
            # Base rank (random surfer)
            rank = (1 - damping) / num_notes

            # Add rank from backlinks
            for source_id in backlinks.get(note_id, []):
                num_outlinks = len(outlinks.get(source_id, []))
                if num_outlinks > 0:
                    rank += damping * (pagerank[source_id] / num_outlinks)

            new_pagerank[note_id] = rank

        pagerank = new_pagerank

    return pagerank


def update_pagerank_scores(adapter, pagerank_scores: Dict[int, float]) -> int:
    """
    Update PageRank scores in storage.

    Args:
        adapter: Storage adapter instance
        pagerank_scores: Dictionary mapping note_id to pagerank score

    Returns:
        Number of notes updated
    """
    updated = 0

    for note_id, score in pagerank_scores.items():
        try:
            success = adapter.update_note(note_id, pagerank=score)
            if success:
                updated += 1
        except Exception as e:
            logger.debug(f"Failed to update PageRank for note {note_id}: {e}")

    return updated


def calculate_and_update_pagerank(adapter) -> int:
    """
    Calculate PageRank and update all notes.

    Args:
        adapter: Storage adapter instance

    Returns:
        Number of notes updated
    """
    logger.info("Calculating PageRank...")
    pagerank_scores = calculate_pagerank(adapter)

    if not pagerank_scores:
        logger.warning("No PageRank scores calculated")
        return 0

    logger.info(f"Updating PageRank for {len(pagerank_scores)} notes...")
    updated = update_pagerank_scores(adapter, pagerank_scores)

    logger.info(f"Updated PageRank for {updated} notes")
    return updated
