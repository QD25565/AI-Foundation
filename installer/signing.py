"""
Ed25519 digital signatures for AI-Foundation release signing.

Uses the `cryptography` library (OpenSSL-backed, battle-tested).
Falls back to subprocess call to a Rust binary if unavailable.

Key management:
  - Release signing key lives in CI secrets (private) + embedded here (public)
  - Each manifest includes a signature over its canonical JSON
  - Verification at install/update time rejects tampered manifests
"""

import json
import os
import subprocess
from pathlib import Path
from typing import Any


# ── Crypto Backend ────────────────────────────────────────

try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import (
        Ed25519PrivateKey,
        Ed25519PublicKey,
    )
    from cryptography.hazmat.primitives.serialization import (
        Encoding,
        NoEncryption,
        PrivateFormat,
        PublicFormat,
    )
    _HAS_CRYPTOGRAPHY = True
except ImportError:
    _HAS_CRYPTOGRAPHY = False


def _ensure_backend():
    if not _HAS_CRYPTOGRAPHY:
        raise RuntimeError(
            "Ed25519 signing requires the 'cryptography' library.\n"
            "Install it with: pip install cryptography"
        )


# ── Key Operations ────────────────────────────────────────

def generate_keypair() -> tuple[bytes, bytes]:
    """
    Generate a new Ed25519 keypair.
    Returns (private_key_raw: 32 bytes, public_key_raw: 32 bytes).
    """
    _ensure_backend()
    private_key = Ed25519PrivateKey.generate()
    priv_bytes = private_key.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption())
    pub_bytes = private_key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    return priv_bytes, pub_bytes


def public_key_from_private(private_raw: bytes) -> bytes:
    """Derive the 32-byte public key from a 32-byte private key."""
    _ensure_backend()
    private_key = Ed25519PrivateKey.from_private_bytes(private_raw)
    return private_key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)


def sign(private_key_raw: bytes, message: bytes) -> bytes:
    """Sign a message. Returns a 64-byte Ed25519 signature."""
    _ensure_backend()
    private_key = Ed25519PrivateKey.from_private_bytes(private_key_raw)
    return private_key.sign(message)


def verify(public_key_raw: bytes, message: bytes, signature: bytes) -> bool:
    """Verify an Ed25519 signature. Returns True if valid."""
    _ensure_backend()
    if len(signature) != 64 or len(public_key_raw) != 32:
        return False
    try:
        public_key = Ed25519PublicKey.from_public_bytes(public_key_raw)
        public_key.verify(signature, message)
        return True
    except Exception:
        return False


# ── Release Signing Helpers ───────────────────────────────

# AI-Foundation release signing public key (hex-encoded, 32 bytes).
# The private key exists ONLY in CI secrets.
# Empty string = no release key configured yet (unsigned manifests allowed).
RELEASE_PUBLIC_KEY_HEX = ""

# Key file names
PRIVATE_KEY_FILENAME = "release-signing.key"
PUBLIC_KEY_FILENAME = "release-signing.pub"


def canonical_manifest_json(manifest: dict[str, Any]) -> bytes:
    """
    Produce a canonical JSON representation of a manifest for signing.
    Deterministic: sorted keys, compact separators, UTF-8 encoded.
    Excludes the 'signature' field to avoid circular dependency.
    """
    signable = {k: v for k, v in manifest.items() if k != "signature"}
    return json.dumps(signable, sort_keys=True, separators=(",", ":")).encode("utf-8")


def sign_manifest(manifest: dict[str, Any], private_key_path: Path) -> str:
    """
    Sign a manifest with the release private key.
    Returns the signature as a hex string.
    """
    if not private_key_path.exists():
        raise FileNotFoundError(f"Private key not found: {private_key_path}")

    private_key_raw = bytes.fromhex(private_key_path.read_text().strip())
    message = canonical_manifest_json(manifest)
    sig = sign(private_key_raw, message)
    return sig.hex()


def verify_manifest(manifest: dict[str, Any], public_key_hex: str | None = None) -> bool:
    """
    Verify a manifest's Ed25519 signature.
    Uses the embedded release public key by default.

    Returns True if:
      - Signature is valid
      - No signature present (unsigned manifests allowed during transition)
      - No public key configured (signing not yet enabled)
    """
    sig_hex = manifest.get("signature")
    if not sig_hex:
        return True  # Unsigned — allowed during transition period

    pub_hex = public_key_hex or RELEASE_PUBLIC_KEY_HEX
    if not pub_hex:
        return True  # No public key configured yet

    try:
        public_key_raw = bytes.fromhex(pub_hex)
        signature = bytes.fromhex(sig_hex)
        message = canonical_manifest_json(manifest)
        return verify(public_key_raw, message, signature)
    except (ValueError, OverflowError):
        return False


def generate_release_keypair(output_dir: Path) -> tuple[Path, Path]:
    """
    Generate a new Ed25519 keypair for release signing.
    Writes private and public keys as hex strings.
    Returns (private_key_path, public_key_path).

    IMPORTANT: The private key must be kept secret (CI secrets only).
    The public key gets embedded in RELEASE_PUBLIC_KEY_HEX above.
    """
    output_dir.mkdir(parents=True, exist_ok=True)

    private_raw, public_raw = generate_keypair()

    priv_path = output_dir / PRIVATE_KEY_FILENAME
    pub_path = output_dir / PUBLIC_KEY_FILENAME

    priv_path.write_text(private_raw.hex() + "\n")
    pub_path.write_text(public_raw.hex() + "\n")

    # Restrict private key permissions on Unix
    try:
        priv_path.chmod(0o600)
    except OSError:
        pass  # Windows doesn't support Unix permissions

    return priv_path, pub_path
