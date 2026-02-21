#!/usr/bin/env python3
"""Test notebook_rs PostgreSQL integration"""

import notebook_core
import traceback
import os

def test_notebook_postgresql():
    print("=" * 60)
    print("NOTEBOOK-RS POSTGRESQL INTEGRATION TEST")
    print("=" * 60)

    # Use PostgreSQL connection string (get from teambook's validated config)
    try:
        from tools.teambook.postgres_auto_config import get_validated_postgres_url
        db_url = get_validated_postgres_url()
    except Exception:
        db_url = os.getenv('POSTGRES_URL', 'postgresql://ai_foundation:ai_foundation@127.0.0.1:5432/ai_foundation')
    print(f"\nDatabase URL: {db_url}")

    try:
        # Create storage
        print("\n1. Creating NotebookStorage with PostgreSQL...")
        storage = notebook_core.NotebookStorage(db_url)
        print("   [OK] Storage created with PostgreSQL backend")

        # Test remember
        print("\n2. Testing remember()...")
        note_id = storage.remember('Test note from Rust PostgreSQL', ['rust', 'postgresql', 'test'])
        print(f"   [OK] Stored note ID: {note_id}")
        assert isinstance(note_id, int), f"Expected int ID, got {type(note_id)}"

        # Test recall without query
        print("\n3. Testing recall() without query...")
        notes = storage.recall()
        print(f"   [OK] Retrieved {len(notes)} notes")
        assert len(notes) > 0, "Expected at least one note"
        print(f"   First note: {notes[0]}")

        # Test recall with query
        print("\n4. Testing recall() with query...")
        searched_notes = storage.recall(query='PostgreSQL', limit=10)
        print(f"   [OK] Query 'PostgreSQL' found {len(searched_notes)} notes")
        assert len(searched_notes) > 0, "Expected to find PostgreSQL note"

        # Test get_note by ID
        print("\n5. Testing get_note()...")
        retrieved = storage.get_note(note_id)
        print(f"   [OK] Retrieved note by ID: {retrieved}")
        assert retrieved is not None, "Expected to retrieve note by ID"
        assert retrieved['id'] == note_id, f"ID mismatch: {retrieved['id']} != {note_id}"

        # Test pin_note
        print("\n6. Testing pin_note()...")
        pinned = storage.pin_note(note_id)
        print(f"   [OK] Pin result: {pinned}")
        assert pinned == True, "Expected pin to succeed"

        # Verify pinned status
        pinned_note = storage.get_note(note_id)
        assert pinned_note['pinned'] == True, "Note should be pinned"
        print(f"   [OK] Note {note_id} is now pinned")

        # Test unpin_note
        print("\n7. Testing unpin_note()...")
        unpinned = storage.unpin_note(note_id)
        print(f"   [OK] Unpin result: {unpinned}")
        assert unpinned == True, "Expected unpin to succeed"

        # Verify unpinned status
        unpinned_note = storage.get_note(note_id)
        assert unpinned_note['pinned'] == False, "Note should be unpinned"
        print(f"   [OK] Note {note_id} is now unpinned")

        # Test get_stats
        print("\n8. Testing get_stats()...")
        stats = storage.get_stats()
        print(f"   [OK] Stats: {stats}")
        assert 'note_count' in stats, "Stats should include note_count"
        assert stats['note_count'] > 0, "Should have at least one note"

        # Test vault operations
        print("\n9. Testing vault_store()...")
        storage.vault_store('test_key', 'test_value_postgresql')
        print("   [OK] Stored vault entry")

        print("\n10. Testing vault_retrieve()...")
        value = storage.vault_retrieve('test_key')
        print(f"   [OK] Retrieved value: {value}")
        assert value == 'test_value_postgresql', f"Value mismatch: {value}"

        print("\n11. Testing vault_list()...")
        keys = storage.vault_list()
        print(f"   [OK] Vault keys: {keys}")
        assert 'test_key' in keys, "Expected to find test_key in vault"

        # Test compression
        print("\n12. Testing compression...")
        large_content = "x" * 15000  # >10KB
        should_compress = notebook_core.should_compress_text(large_content)
        print(f"   Should compress 15KB content: {should_compress}")
        assert should_compress == True, "15KB content should compress"

        compressed = notebook_core.compress_text(large_content)
        print(f"   Compressed size: {len(compressed)} bytes")
        assert len(compressed) < len(large_content), "Compressed should be smaller"

        decompressed = notebook_core.decompress_text(compressed)
        print(f"   Decompressed matches: {decompressed == large_content}")
        assert decompressed == large_content, "Decompression should match original"

        # Test storing compressed note
        print("\n13. Testing compressed note storage...")
        large_note_id = storage.remember(large_content, ['large', 'compressed'])
        print(f"   [OK] Stored large note ID: {large_note_id}")

        large_note = storage.get_note(large_note_id)
        assert large_note['compressed'] == True, "Large note should be marked compressed"
        print(f"   [OK] Large note correctly marked as compressed")

        # Test session management
        print("\n14. Testing get_or_create_session()...")
        session_id = storage.get_or_create_session('resonance-684')
        print(f"   [OK] Session ID: {session_id}")
        assert isinstance(session_id, int), "Session ID should be integer"

        # Second call should return same session
        session_id2 = storage.get_or_create_session('resonance-684')
        assert session_id == session_id2, "Should return same session for same agent"
        print(f"   [OK] Session persistence verified")

        print("\n" + "=" * 60)
        print("ALL TESTS PASSED!")
        print("=" * 60)
        print("\n[SUMMARY]")
        print(f"  - PostgreSQL backend: ✅ Working")
        print(f"  - Note operations: ✅ remember, recall, get_note")
        print(f"  - Pin operations: ✅ pin_note, unpin_note")
        print(f"  - Vault operations: ✅ store, retrieve, list")
        print(f"  - Stats: ✅ get_stats")
        print(f"  - Compression: ✅ compress, decompress, auto-compress")
        print(f"  - Sessions: ✅ get_or_create_session")
        print(f"\n[READY FOR DEPLOYMENT] notebook-rs PostgreSQL backend is production-ready!")

    except Exception as e:
        print(f"\n[ERROR] {e}")
        traceback.print_exc()
        return False

    return True

if __name__ == "__main__":
    success = test_notebook_postgresql()
    exit(0 if success else 1)
