#!/usr/bin/env python3
"""
AI-Foundation Release Signing
==============================
Generates and signs release manifests with SHA256 hashes + Ed25519 signatures.
Run this after building binaries, before distributing.

Usage:
    python sign.py                              # Generate manifest for bin/windows/
    python sign.py --sign --key release.key     # Generate + sign with Ed25519
    python sign.py --verify                     # Verify manifest (hashes + signature)
    python sign.py --keygen                     # Generate new Ed25519 signing keypair

The manifest is written to the binary directory as manifest.json.
"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from installer import manifest, signing
from installer.ui import G, ok, info, warn, error, step, tree_row


REPO_ROOT = Path(__file__).parent
VERSION_FILE = REPO_ROOT / "version.txt"


def get_version() -> str:
    if VERSION_FILE.exists():
        return VERSION_FILE.read_text().strip()
    return "unknown"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="sign.py",
        description="Generate signed release manifest for AI-Foundation binaries.",
    )
    parser.add_argument(
        "--bin-dir", metavar="PATH",
        help="Directory containing built binaries (default: bin/windows/)"
    )
    parser.add_argument(
        "--channel", default="stable",
        choices=["stable", "beta", "nightly"],
        help="Release channel (default: stable)"
    )
    parser.add_argument(
        "--min-daemon", metavar="VER",
        help="Minimum compatible daemon version"
    )
    parser.add_argument(
        "--verify", action="store_true",
        help="Verify existing manifest (hashes + Ed25519 signature)"
    )
    parser.add_argument(
        "--output", metavar="PATH",
        help="Write manifest to custom path instead of bin-dir"
    )
    parser.add_argument(
        "--sign", action="store_true",
        help="Sign the manifest with Ed25519 (requires --key)"
    )
    parser.add_argument(
        "--key", metavar="PATH",
        help="Path to Ed25519 private key file (hex-encoded, 32 bytes)"
    )
    parser.add_argument(
        "--pub-key", metavar="HEX",
        help="Public key hex for verification (overrides embedded key)"
    )
    parser.add_argument(
        "--keygen", action="store_true",
        help="Generate a new Ed25519 signing keypair"
    )
    parser.add_argument(
        "--keygen-dir", metavar="PATH", default=".",
        help="Directory to write generated keypair (default: current directory)"
    )
    return parser.parse_args()


def do_keygen(args: argparse.Namespace) -> int:
    """Generate a new Ed25519 signing keypair."""
    output_dir = Path(args.keygen_dir)

    step("Generating Ed25519 Signing Keypair")

    priv_path, pub_path = signing.generate_release_keypair(output_dir)
    pub_hex = pub_path.read_text().strip()

    ok(f"Private key: {priv_path}")
    ok(f"Public key:  {pub_path}")
    print()
    info(f"Public key (hex): {pub_hex}")
    print()
    warn("KEEP THE PRIVATE KEY SECRET — store in CI secrets only.")
    info("Embed the public key hex in installer/signing.py RELEASE_PUBLIC_KEY_HEX")
    info(f"and in install.sh / install.ps1 for bootstrap verification.")

    return 0


def do_generate(args: argparse.Namespace) -> int:
    version = get_version()

    bin_dir = Path(args.bin_dir) if args.bin_dir else REPO_ROOT / "bin" / "windows"
    if not bin_dir.exists():
        error(f"Binary directory not found: {bin_dir}")
        return 1

    step(f"Generating manifest v{version} ({args.channel})")
    info(f"Source: {bin_dir}")

    m = manifest.generate(
        bin_dir=bin_dir,
        version=version,
        channel=args.channel,
        min_daemon_version=args.min_daemon,
    )

    binary_count = len(m["binaries"])
    if binary_count == 0:
        error("No binaries found in directory")
        return 1

    # Sign if requested
    if args.sign:
        if not args.key:
            error("--sign requires --key <path-to-private-key>")
            return 1
        key_path = Path(args.key)
        if not key_path.exists():
            error(f"Private key not found: {key_path}")
            return 1

        sig_hex = signing.sign_manifest(m, key_path)
        m["signature"] = sig_hex
        ok(f"Signed with Ed25519 (sig: {sig_hex[:16]}...)")

    # Write manifest
    output = Path(args.output) if args.output else bin_dir
    path = manifest.write(m, output)
    ok(f"Manifest written: {path}")

    # Summary
    print()
    tree_row("Version", f"v{version}")
    tree_row("Channel", args.channel)
    tree_row("Signed", "Yes (Ed25519)" if m.get("signature") else "No")
    tree_row("Binaries", str(binary_count))

    total_size = sum(b["size"] for b in m["binaries"].values())
    tree_row("Total size", f"{total_size / (1024 * 1024):.1f} MB", is_last=True)

    print()
    for name, entry in sorted(m["binaries"].items()):
        size_kb = entry["size"] / 1024
        hash_short = entry["sha256"][:12]
        ok(f"  {name}: {size_kb:.0f}K  [{hash_short}...]")

    return 0


def do_verify(args: argparse.Namespace) -> int:
    bin_dir = Path(args.bin_dir) if args.bin_dir else REPO_ROOT / "bin" / "windows"

    step("Verifying manifest")

    m = manifest.load(bin_dir)
    if m is None:
        error(f"No manifest found in {bin_dir}")
        info("Run 'python sign.py' to generate one")
        return 1

    info(f"Manifest version: v{m.get('version', '?')}")
    info(f"Channel: {m.get('channel', '?')}")
    info(f"Published: {m.get('pub_date', '?')}")

    # Ed25519 signature verification
    if m.get("signature"):
        pub_hex = args.pub_key if args.pub_key else None
        if signing.verify_manifest(m, pub_hex):
            ok("Ed25519 signature: VALID")
        else:
            error("Ed25519 signature: INVALID")
            error("Manifest may have been tampered with!")
            return 1
    else:
        info("Ed25519 signature: not present (unsigned manifest)")

    print()

    # Binary hash verification
    all_ok, messages = manifest.verify_all(bin_dir, m)

    for msg in messages:
        if "MISSING" in msg or "mismatch" in msg:
            error(f"  {msg}")
        elif "verified" in msg:
            ok(f"  {msg}")
        else:
            info(f"  {msg}")

    print()
    if all_ok:
        ok(f"All {len(messages)} binaries verified")
    else:
        error("Verification FAILED — some binaries do not match manifest")

    return 0 if all_ok else 1


def main() -> int:
    args = parse_args()
    if args.keygen:
        return do_keygen(args)
    if args.verify:
        return do_verify(args)
    return do_generate(args)


if __name__ == "__main__":
    sys.exit(main())
