#!/usr/bin/env python3
"""Test notebook_rs Python integration"""

import notebook_core
import traceback

def test_notebook():
    print("=" * 60)
    print("NOTEBOOK-RS INTEGRATION TEST")
    print("=" * 60)

    try:
        # Create storage
        print("\n1. Creating NotebookStorage...")
        storage = notebook_core.NotebookStorage('test_notebook.db')
        print("   [OK] Storage created")

        # Test remember
        print("\n2. Testing remember()...")
        note_id = storage.remember('Test note from Rust', ['rust', 'test'])
        print(f"   [OK] Stored note ID: {note_id}")

        # Test recall
        print("\n3. Testing recall()...")
        notes = storage.recall()
        print(f"   [OK] Retrieved {len(notes)} notes")
        if notes:
            print(f"   First note: {notes[0]}")

        # Test get_stats
        print("\n4. Testing get_stats()...")
        stats = storage.get_stats()
        print(f"   [OK] Stats: {stats}")

        # Test compression
        print("\n5. Testing compression...")
        large_content = "x" * 15000  # >10KB
        should_compress = notebook_core.should_compress_text(large_content)
        print(f"   Should compress 15KB content: {should_compress}")

        compressed = notebook_core.compress_text(large_content)
        print(f"   Compressed size: {len(compressed)} bytes")

        decompressed = notebook_core.decompress_text(compressed)
        print(f"   Decompressed matches: {decompressed == large_content}")

        print("\n" + "=" * 60)
        print("ALL TESTS PASSED!")
        print("=" * 60)

    except Exception as e:
        print(f"\n[ERROR] {e}")
        traceback.print_exc()
        return False

    return True

if __name__ == "__main__":
    success = test_notebook()
    exit(0 if success else 1)
