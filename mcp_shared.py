#!/usr/bin/env python3
"""
MCP Shared Utilities v1.0.0
============================
Common utilities for all MCP tools to reduce boilerplate and ensure consistency.
Provides: identity management, logging, data paths, formatting, and server helpers.
"""

import os
import sys
import json
import random
import re
import logging
import base64
import tempfile
import hashlib
from dataclasses import dataclass, field
from pathlib import Path
from datetime import datetime, timezone
from threading import Lock
from typing import Optional, Dict, Any, Mapping, MutableMapping, Callable

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)


_TOOL_NAME_PATTERN = re.compile(r"^[A-Za-z0-9_-]{1,64}$")

# ============= VERSION & CONFIGURATION =============
MCP_SHARED_VERSION = "1.0.0"

# ============= CROSS-PLATFORM DATA DIRECTORY =============
def get_base_data_dir() -> Path:
    """Get cross-platform base directory for all MCP tools"""
    if sys.platform == "win32":
        base = Path.home() / "AppData" / "Roaming" / "Claude" / "tools"
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support" / "Claude" / "tools"
    else:
        base = Path.home() / ".claude" / "tools"
    
    # Allow override via environment
    if 'MCP_DATA_DIR' in os.environ:
        base = Path(os.environ['MCP_DATA_DIR'])
    
    base.mkdir(parents=True, exist_ok=True)
    return base

BASE_DATA_DIR = get_base_data_dir()

def _get_tool_data_dir(tool_name: str) -> Path:
    """Get data directory for specific tool (internal)"""
    tool_dir = BASE_DATA_DIR / f"{tool_name}_data"
    tool_dir.mkdir(parents=True, exist_ok=True)
    return tool_dir

# Keep public alias for backwards compatibility
def get_tool_data_dir(tool_name: str) -> Path:
    """Get data directory for specific tool"""
    return _get_tool_data_dir(tool_name)

# ============= LOGGING SETUP =============
def setup_logging(tool_name: str, level=logging.WARNING):
    """Configure logging to stderr only (MCP requirement)

    Default level is WARNING to reduce noise in normal operation.
    Set level=logging.INFO or logging.DEBUG for verbose output.
    """
    logging.basicConfig(
        level=level,
        format=f'%(asctime)s - [{tool_name}] - %(message)s',
        stream=sys.stderr
    )
    return logging.getLogger(tool_name)

# ============= AI IDENTITY MANAGEMENT =============

IDENTITY_METADATA_VERSION = 2


@dataclass(frozen=True)
class AIIdentity:
    """Structured representation of an AI identity."""

    ai_id: str
    display_name: str
    handle: str
    fingerprint: str
    public_key: str
    created_at: str
    updated_at: str
    metadata_version: int = IDENTITY_METADATA_VERSION
    source: str = "generated"
    protocol_handles: Mapping[str, str] = field(default_factory=dict)

    def as_dict(self) -> Dict[str, Any]:
        payload: Dict[str, Any] = {
            "version": self.metadata_version,
            "ai_id": self.ai_id,
            "display_name": self.display_name,
            "handle": self.handle,
            "fingerprint": self.fingerprint,
            "public_key": self.public_key,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "source": self.source,
        }

        if self.protocol_handles:
            payload["protocol_handles"] = dict(self.protocol_handles)

        return payload

    def resolve_handle(
        self,
        protocol: Optional[str] = None,
        *,
        prefer_pretty: bool = False,
        capabilities: Optional[Mapping[str, Any]] = None,
        fallback: bool = True,
    ) -> str:
        """Return the best handle for a protocol/capability set."""

        handles: Dict[str, str] = dict(self.protocol_handles or {})
        normalized_protocol = (protocol or "").strip().lower()

        def _candidate_allowed(candidate: Optional[str]) -> bool:
            if not candidate:
                return False
            if not capabilities:
                return True

            pattern = capabilities.get("pattern")
            if isinstance(pattern, str) and pattern:
                try:
                    if not re.match(pattern, candidate):
                        return False
                except re.error:
                    logging.debug("Invalid pattern '%s' supplied in capabilities", pattern)
                    return False

            max_length = capabilities.get("max_length")
            if isinstance(max_length, str) and max_length.isdigit():
                max_length = int(max_length)
            if isinstance(max_length, int) and max_length > 0:
                if len(candidate) > max_length:
                    return False

            supports_spaces = capabilities.get("supports_spaces", True)
            if isinstance(supports_spaces, str):
                supports_spaces = supports_spaces.lower() not in {"0", "false", "no", "off"}
            if not supports_spaces and " " in candidate:
                return False

            supports_unicode = capabilities.get("supports_unicode", True)
            if isinstance(supports_unicode, str):
                supports_unicode = supports_unicode.lower() not in {"0", "false", "no", "off"}
            if not supports_unicode:
                try:
                    candidate.encode("ascii")
                except UnicodeEncodeError:
                    return False

            prefer_ascii = capabilities.get("prefer_ascii", False)
            if isinstance(prefer_ascii, str):
                prefer_ascii = prefer_ascii.lower() not in {"0", "false", "no", "off"}
            if prefer_ascii:
                try:
                    candidate.encode("ascii")
                except UnicodeEncodeError:
                    return False

            return True

        def _pick(*options: Optional[str]) -> Optional[str]:
            for option in options:
                if option and _candidate_allowed(option):
                    return option
            return None

        aliases = {
            "mcp": ("remote", "api"),
            "http": ("web", "rest"),
            "cli": ("terminal", "shell"),
        }

        if prefer_pretty:
            pretty = handles.get("pretty") or self.handle
            pretty_choice = _pick(pretty)
            if pretty_choice and (not normalized_protocol or normalized_protocol in {"cli", "terminal"}):
                return pretty_choice

        if normalized_protocol:
            specific = _pick(handles.get(normalized_protocol))
            if specific:
                return specific

            for alias in aliases.get(normalized_protocol, ()):  # type: ignore[assignment]
                alias_choice = _pick(handles.get(alias))
                if alias_choice:
                    return alias_choice

        default_choice = _pick(handles.get("default"), handles.get("slug"))
        if default_choice:
            return default_choice

        if fallback:
            sanitized = _build_protocol_handles(self.display_name, self.ai_id.split("-")[-1], handles).get("default")
            fallback_choice = _pick(sanitized, self.handle)
            if fallback_choice:
                return fallback_choice

        return self.handle


_IDENTITY_DIR: Optional[Path] = None
_IDENTITY_CACHE: Optional[AIIdentity] = None
_PRIVATE_KEY_CACHE: Optional[Ed25519PrivateKey] = None
_REGISTRY_CACHE: Optional[Dict[str, Any]] = None
_REGISTRY_LOCK = Lock()


def _ensure_identity_dir() -> Path:
    """Return the directory used for identity metadata, creating it if needed."""

    global _IDENTITY_DIR

    if _IDENTITY_DIR is not None:
        return _IDENTITY_DIR

    override = os.environ.get("AI_IDENTITY_DIR")
    if override:
        identity_dir = Path(override).expanduser()
    else:
        identity_dir = BASE_DATA_DIR / "identity"

    identity_dir.mkdir(parents=True, exist_ok=True)
    _IDENTITY_DIR = identity_dir
    return identity_dir


def _identity_metadata_path() -> Path:
    return _ensure_identity_dir() / "ai_identity.json"


def _identity_private_key_path() -> Path:
    return _ensure_identity_dir() / "ai_identity_private.key"


def _identity_registry_path() -> Path:
    override = os.environ.get("AI_IDENTITY_REGISTRY")
    if override:
        path = Path(override).expanduser()
    else:
        path = BASE_DATA_DIR / "identity_registry.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def _slugify_name(name: str) -> str:
    base = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return base or "ai"


def _normalize_display_name(raw_name: Optional[str]) -> str:
    cleaned = str(raw_name or "").strip()
    if not cleaned:
        return "AI"

    # Replace underscores and collapse whitespace
    cleaned = cleaned.replace("_", " ")
    cleaned = re.sub(r"\s+", " ", cleaned)

    # Allow legacy hyphenated identifiers to become friendly names
    if "-" in cleaned and not any(ch.isspace() for ch in cleaned):
        parts = [part for part in cleaned.split("-") if part]
        if parts:
            cleaned = " ".join(parts)

    return cleaned.strip()


def _suffix_from_public_key(public_bytes: bytes) -> str:
    digest = hashlib.sha3_256(public_bytes).digest()
    numeric = int.from_bytes(digest[:3], "big") % 1000
    return f"{numeric:03d}"


def _fingerprint_from_public_key(public_bytes: bytes) -> str:
    return hashlib.sha3_256(public_bytes).hexdigest()[:16].upper()


def _build_protocol_handles(
    display_name: str,
    suffix: str,
    existing: Optional[Mapping[str, str]] = None,
) -> Dict[str, str]:
    """Construct protocol-safe handle variants."""

    friendly = (display_name or "AI").strip() or "AI"
    slug = _slugify_name(display_name)
    canonical = f"{slug}-{suffix}" if suffix else slug
    canonical = re.sub(r"[^A-Za-z0-9_-]", "-", canonical)
    canonical = re.sub(r"-{2,}", "-", canonical).strip("-") or "ai"
    if len(canonical) > 64:
        canonical = canonical[:64].rstrip("-") or canonical[:64]

    handles: Dict[str, str] = {
        "pretty": f"{friendly} ({suffix})" if suffix else friendly,
        "slug": canonical,
        "default": canonical,
        "mcp": canonical,
        "remote": canonical,
        "api": canonical,
        "http": canonical,
        "web": canonical,
        "rest": canonical,
        "cli": f"{friendly} ({suffix})" if suffix else friendly,
        "terminal": f"{friendly} ({suffix})" if suffix else friendly,
        "shell": f"{friendly} ({suffix})" if suffix else friendly,
    }

    if existing:
        for key, value in existing.items():
            if isinstance(key, str) and isinstance(value, str) and value.strip():
                handles[key.strip().lower()] = value.strip()

    return handles


def _write_json_secure(path: Path, payload: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_path = tempfile.mkstemp(prefix=path.name, dir=str(path.parent))
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, indent=2, sort_keys=True)
        os.replace(tmp_path, path)
        if os.name == "posix":
            os.chmod(path, 0o600)
    finally:
        try:
            if os.path.exists(tmp_path):
                os.remove(tmp_path)
        except OSError:
            pass


def _read_json(path: Path) -> Optional[Dict[str, Any]]:
    if not path.exists():
        return None
    try:
        with path.open("r", encoding="utf-8") as handle:
            return json.load(handle)
    except Exception as exc:
        logging.warning(f"Failed to read identity metadata {path}: {exc}")
        return None


def _write_private_key(private_key: Ed25519PrivateKey, path: Path) -> None:
    private_bytes = private_key.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )
    encoded = base64.b64encode(private_bytes).decode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.write(fd, encoded.encode("utf-8"))
    finally:
        os.close(fd)
    if os.name == "posix":
        os.chmod(path, 0o600)


def _read_private_key(path: Path) -> Optional[Ed25519PrivateKey]:
    if not path.exists():
        return None
    try:
        encoded = path.read_text(encoding="utf-8").strip()
        if not encoded:
            return None
        private_bytes = base64.b64decode(encoded.encode("utf-8"))
        return Ed25519PrivateKey.from_private_bytes(private_bytes)
    except Exception as exc:
        logging.warning(f"Failed to read identity private key {path}: {exc}")
        return None


def _legacy_identity_paths() -> Mapping[str, Path]:
    base_paths = {
        "module": Path(__file__).parent / "ai_identity.txt",
        "data": BASE_DATA_DIR / "ai_identity.txt",
        "home": Path.home() / "ai_identity.txt",
    }
    env_path = os.environ.get("AI_IDENTITY_LEGACY_FILE")
    if env_path:
        base_paths["env"] = Path(env_path).expanduser()
    return base_paths


def _determine_display_name() -> str:
    preferred = os.environ.get("AI_DISPLAY_NAME") or os.environ.get("AI_NAME")
    if preferred:
        return _normalize_display_name(preferred)

    for label, legacy_path in _legacy_identity_paths().items():
        try:
            if legacy_path.exists():
                stored = legacy_path.read_text(encoding="utf-8").strip()
                if stored:
                    name = _normalize_display_name(stored)
                    if name:
                        return name
        except Exception as exc:
            logging.debug(f"Unable to read legacy identity {label}: {exc}")

    adjectives = ["Swift", "Bright", "Sharp", "Quick", "Clear", "Deep", "Keen", "Pure"]
    nouns = ["Mind", "Spark", "Flow", "Core", "Sync", "Node", "Wave", "Link"]
    fallback = f"{random.choice(adjectives)} {random.choice(nouns)}"
    return _normalize_display_name(fallback)


def _generate_identity(display_name: str) -> AIIdentity:
    private_key = Ed25519PrivateKey.generate()
    public_key = private_key.public_key()
    public_bytes = public_key.public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    suffix = _suffix_from_public_key(public_bytes)
    fingerprint = _fingerprint_from_public_key(public_bytes)
    slug = _slugify_name(display_name)
    ai_id = f"{slug}-{suffix}"
    friendly_name = display_name.strip() or "AI"
    protocol_handles = _build_protocol_handles(friendly_name, suffix)
    handle = protocol_handles.get("pretty", f"{friendly_name} ({suffix})")
    now = datetime.now(timezone.utc).isoformat()
    identity = AIIdentity(
        ai_id=ai_id,
        display_name=friendly_name,
        handle=handle,
        fingerprint=fingerprint,
        public_key=base64.b64encode(public_bytes).decode("utf-8"),
        created_at=now,
        updated_at=now,
        source="generated",
        protocol_handles=protocol_handles,
    )

    _write_json_secure(_identity_metadata_path(), identity.as_dict())
    _write_private_key(private_key, _identity_private_key_path())
    _write_legacy_identity_files(identity)
    return identity


def _write_legacy_identity_files(identity: AIIdentity) -> None:
    legacy_value = identity.handle
    for legacy_path in _legacy_identity_paths().values():
        try:
            legacy_path.parent.mkdir(parents=True, exist_ok=True)
            fd = os.open(legacy_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            try:
                os.write(fd, legacy_value.encode("utf-8"))
            finally:
                os.close(fd)
            if os.name == "posix":
                os.chmod(legacy_path, 0o600)
        except Exception as exc:
            logging.debug(f"Unable to update legacy identity file {legacy_path}: {exc}")


def _load_identity_from_metadata(metadata: Mapping[str, Any], source: str) -> Optional[AIIdentity]:
    if not isinstance(metadata, Mapping):
        return None

    display_name = _normalize_display_name(metadata.get("display_name") or metadata.get("name"))
    public_key_b64 = metadata.get("public_key")

    if not isinstance(public_key_b64, str):
        return None

    try:
        public_bytes = base64.b64decode(public_key_b64.encode("utf-8"))
    except Exception:
        logging.warning(f"Identity metadata {source} has invalid public key")
        return None

    suffix = _suffix_from_public_key(public_bytes)
    fingerprint = _fingerprint_from_public_key(public_bytes)
    ai_id_candidate = metadata.get("ai_id") or metadata.get("slug")
    if isinstance(ai_id_candidate, str):
        ai_id_candidate = ai_id_candidate.strip().lower()
    else:
        ai_id_candidate = None

    slug = _slugify_name(display_name)
    ai_id = ai_id_candidate or f"{slug}-{suffix}"

    handles_raw = metadata.get("protocol_handles") if isinstance(metadata, Mapping) else None
    protocol_handles: Dict[str, str] = {}
    if isinstance(handles_raw, Mapping):
        for key, value in handles_raw.items():
            if isinstance(key, str) and isinstance(value, str) and value.strip():
                protocol_handles[key.strip().lower()] = value.strip()

    protocol_handles = _build_protocol_handles(display_name, suffix, protocol_handles)
    handle = protocol_handles.get("pretty") or metadata.get("handle")
    if not isinstance(handle, str) or not handle.strip():
        handle = f"{display_name} ({suffix})"

    created_at = metadata.get("created_at") or datetime.now(timezone.utc).isoformat()
    updated_at = metadata.get("updated_at") or created_at

    return AIIdentity(
        ai_id=ai_id,
        display_name=display_name,
        handle=handle.strip(),
        fingerprint=fingerprint,
        public_key=base64.b64encode(public_bytes).decode("utf-8"),
        created_at=str(created_at),
        updated_at=str(updated_at),
        metadata_version=int(metadata.get("version") or IDENTITY_METADATA_VERSION),
        source=source,
        protocol_handles=protocol_handles,
    )


def _load_existing_identity() -> Optional[AIIdentity]:
    candidates = []

    env_metadata = os.environ.get("AI_IDENTITY_FILE")
    if env_metadata:
        candidates.append(Path(env_metadata).expanduser())

    candidates.append(_identity_metadata_path())
    candidates.append(Path(__file__).parent / "ai_identity.json")
    candidates.append(BASE_DATA_DIR / "ai_identity.json")
    candidates.append(Path.home() / "ai_identity.json")

    for candidate in candidates:
        metadata = _read_json(candidate)
        if not metadata:
            continue
        identity = _load_identity_from_metadata(metadata, str(candidate))
        if identity:
            # Ensure canonical copy is stored in the managed location
            if candidate != _identity_metadata_path():
                try:
                    _write_json_secure(_identity_metadata_path(), identity.as_dict())
                except Exception as exc:
                    logging.warning(f"Failed to persist canonical identity file: {exc}")
            return identity

    return None


def _load_identity(refresh: bool = False) -> AIIdentity:
    global _IDENTITY_CACHE, _PRIVATE_KEY_CACHE

    if _IDENTITY_CACHE is not None and not refresh:
        return _IDENTITY_CACHE

    identity = _load_existing_identity()
    private_key = _read_private_key(_identity_private_key_path()) if identity else None

    if identity and private_key:
        _IDENTITY_CACHE = identity
        _PRIVATE_KEY_CACHE = private_key
        _write_legacy_identity_files(identity)
        return identity

    display_name = identity.display_name if identity else _determine_display_name()

    generated_identity = _generate_identity(display_name)
    _IDENTITY_CACHE = generated_identity
    _PRIVATE_KEY_CACHE = _read_private_key(_identity_private_key_path())
    if _PRIVATE_KEY_CACHE is None:
        # Regenerate if private key could not be persisted
        private_key = Ed25519PrivateKey.generate()
        _write_private_key(private_key, _identity_private_key_path())
        _PRIVATE_KEY_CACHE = private_key
    return generated_identity


def _save_registry(registry: Dict[str, Any]) -> None:
    _write_json_secure(_identity_registry_path(), registry)


def load_identity_registry(refresh: bool = False) -> Dict[str, Any]:
    global _REGISTRY_CACHE

    if _REGISTRY_CACHE is not None and not refresh:
        return _REGISTRY_CACHE

    data = _read_json(_identity_registry_path()) or {}
    if not isinstance(data, dict):
        data = {}
    _REGISTRY_CACHE = data
    return data


def register_identity(identity: AIIdentity) -> Dict[str, Any]:
    with _REGISTRY_LOCK:
        registry = load_identity_registry(refresh=True)
        entry = registry.get(identity.ai_id, {})

        created_at = entry.get("created_at") or identity.created_at
        updated_at = datetime.now(timezone.utc).isoformat()

        entry = {
            "ai_id": identity.ai_id,
            "display_name": identity.display_name,
            "handle": identity.handle,
            "fingerprint": identity.fingerprint,
            "public_key": identity.public_key,
            "created_at": created_at,
            "updated_at": updated_at,
        }

        if identity.protocol_handles:
            entry["protocol_handles"] = dict(identity.protocol_handles)

        registry[identity.ai_id] = entry
        _REGISTRY_CACHE = registry
        _save_registry(registry)
        return entry


def get_identity_registry_entry(ai_id: str) -> Optional[Dict[str, Any]]:
    registry = load_identity_registry()
    entry = registry.get(ai_id)
    if entry:
        return dict(entry)
    return None


def get_current_ai_identity() -> Dict[str, Any]:
    identity = _load_identity()
    return identity.as_dict()


def get_ai_private_key() -> Optional[Ed25519PrivateKey]:
    global _PRIVATE_KEY_CACHE

    if _PRIVATE_KEY_CACHE is not None:
        return _PRIVATE_KEY_CACHE

    key = _read_private_key(_identity_private_key_path())
    if key is not None:
        _PRIVATE_KEY_CACHE = key
    return key


def sign_with_current_identity(payload: bytes) -> Optional[str]:
    key = get_ai_private_key()
    if key is None:
        return None
    signature = key.sign(payload)
    return base64.b64encode(signature).decode("utf-8")


def verify_signature(public_key_b64: str, payload: bytes, signature_b64: str) -> bool:
    try:
        public_bytes = base64.b64decode(public_key_b64.encode("utf-8"))
        signature = base64.b64decode(signature_b64.encode("utf-8"))
        public_key = Ed25519PublicKey.from_public_bytes(public_bytes)
        public_key.verify(signature, payload)
        return True
    except (ValueError, InvalidSignature, TypeError) as exc:
        logging.debug(f"Signature verification failed: {exc}")
        return False


def fingerprint_for_public_key(public_key_b64: str) -> Optional[str]:
    try:
        public_bytes = base64.b64decode(public_key_b64.encode("utf-8"))
    except Exception:
        return None
    return _fingerprint_from_public_key(public_bytes)


def get_protocol_handle_map() -> Dict[str, str]:
    """Return a copy of the protocol handle mapping for the current AI."""

    return dict(AI_IDENTITY.protocol_handles)


def resolve_identity_label(
    protocol: Optional[str] = None,
    *,
    prefer_pretty: bool = False,
    capabilities: Optional[Mapping[str, Any]] = None,
    fallback: bool = True,
) -> str:
    """Resolve the optimal identity label for the requested protocol."""

    effective_capabilities: Optional[Dict[str, Any]]
    if capabilities:
        effective_capabilities = dict(capabilities)
    else:
        effective_capabilities = {}

    if prefer_pretty and not supports_emoji():
        effective_capabilities.setdefault("prefer_ascii", True)
        effective_capabilities.setdefault("supports_unicode", False)

    if not effective_capabilities:
        effective_capabilities = None

    return AI_IDENTITY.resolve_handle(
        protocol,
        prefer_pretty=prefer_pretty,
        capabilities=effective_capabilities,
        fallback=fallback,
    )


def get_resolved_handle_map(
    protocol_capabilities: Optional[Mapping[str, Mapping[str, Any]]] = None,
    *,
    include_pretty: bool = True,
    include_ascii: bool = True,
) -> Dict[str, str]:
    """Return a protocol-aware handle map with environment fallbacks."""

    protocol_capabilities = dict(protocol_capabilities or {})
    resolved: Dict[str, str] = {}

    preferred_pretty_protocols = {"cli", "terminal", "shell"}
    default_protocols = ["cli", "mcp", "http", "api", "remote"]

    seen = set()
    for proto in default_protocols:
        seen.add(proto)
        caps = protocol_capabilities.get(proto)
        handle = resolve_identity_label(
            proto,
            prefer_pretty=proto in preferred_pretty_protocols,
            capabilities=caps,
            fallback=True,
        )
        if handle:
            resolved[proto] = handle

    # Include any additional custom protocols requested by callers
    for proto, caps in protocol_capabilities.items():
        if proto in seen:
            continue
        handle = resolve_identity_label(
            proto,
            prefer_pretty=proto in preferred_pretty_protocols,
            capabilities=caps,
            fallback=True,
        )
        if handle:
            resolved[proto] = handle

    if include_pretty:
        resolved["pretty"] = resolve_identity_label(None, prefer_pretty=True, fallback=True)

    if include_ascii:
        ascii_capabilities = {"prefer_ascii": True, "supports_unicode": False}
        resolved["ascii"] = resolve_identity_label(None, capabilities=ascii_capabilities, fallback=True)

    return {key: value for key, value in resolved.items() if value}


_EMOJI_SUPPORT: Optional[bool] = None


def supports_emoji() -> bool:
    """Detect whether the current environment can safely render emoji."""

    global _EMOJI_SUPPORT

    if _EMOJI_SUPPORT is not None:
        return _EMOJI_SUPPORT

    disable_env = os.environ.get("TEAMBOOK_FORCE_ASCII") or os.environ.get("MCP_FORCE_ASCII")
    if isinstance(disable_env, str) and disable_env.lower() in {"1", "true", "yes", "on"}:
        _EMOJI_SUPPORT = False
        return _EMOJI_SUPPORT

    encoding_candidates = [
        getattr(sys.stdout, "encoding", None),
        getattr(sys.stderr, "encoding", None),
        sys.getdefaultencoding(),
        "utf-8",
    ]

    for encoding in encoding_candidates:
        if not encoding:
            continue
        try:
            "🟢".encode(encoding)
            _EMOJI_SUPPORT = True
            break
        except UnicodeEncodeError:
            continue
        except LookupError:
            continue

    if _EMOJI_SUPPORT is None:
        _EMOJI_SUPPORT = False

    return _EMOJI_SUPPORT


def validate_tool_name(name: str) -> str:
    """Ensure tool names comply with strict MCP-safe patterns."""

    candidate = str(name or "").strip()
    if not candidate:
        raise ValueError("Tool name cannot be empty")

    if not _TOOL_NAME_PATTERN.fullmatch(candidate):
        raise ValueError(
            f"Tool name '{name}' must match pattern {_TOOL_NAME_PATTERN.pattern}"
        )

    return candidate


def audit_tool_registry(
    tool_map: Mapping[str, Any], *, allow_non_callable: bool = False
) -> Dict[str, Any]:
    """Validate tool registry keys and ensure handlers are callable."""

    validated: Dict[str, Any] = {}

    for raw_name, handler in tool_map.items():
        safe_name = validate_tool_name(raw_name)

        if not allow_non_callable and not callable(handler):
            raise TypeError(f"Tool '{safe_name}' handler must be callable")

        if safe_name in validated and validated[safe_name] is not handler:
            raise ValueError(f"Duplicate handler registration for '{safe_name}'")

        validated[safe_name] = handler

    return validated


def validate_tool_schemas(tool_schemas: Mapping[str, Mapping[str, Any]]) -> Dict[str, Mapping[str, Any]]:
    """Return a copy of tool schemas with enforced naming guarantees."""

    sanitized: Dict[str, Mapping[str, Any]] = {}

    for name, schema in tool_schemas.items():
        safe_name = validate_tool_name(name)
        sanitized[safe_name] = dict(schema)

    return sanitized


def get_persistent_id() -> str:
    identity = _load_identity()
    register_identity(identity)
    return identity.ai_id


# Get AI ID and extended identity information from environment or persistent storage
AI_IDENTITY = _load_identity()
register_identity(AI_IDENTITY)
CURRENT_AI_ID = os.environ.get('AI_ID', AI_IDENTITY.ai_id)
CURRENT_AI_DISPLAY_NAME = AI_IDENTITY.display_name
CURRENT_AI_HANDLE = AI_IDENTITY.handle
CURRENT_AI_PUBLIC_KEY = AI_IDENTITY.public_key
CURRENT_AI_FINGERPRINT = AI_IDENTITY.fingerprint
CURRENT_AI_PROTOCOL_HANDLES = dict(AI_IDENTITY.protocol_handles)

# ============= PARAMETER NORMALIZATION =============
def _normalize_param(value: Any) -> Any:
    """Normalize parameter values - convert string 'null' to None for forgiving tool calls (internal)"""
    if value == 'null' or value == 'None':
        return None
    return value

# Keep public alias for backwards compatibility
def normalize_param(value: Any) -> Any:
    """Normalize parameter values - convert string 'null' to None for forgiving tool calls"""
    return _normalize_param(value)

# ============= OUTPUT FORMATTING =============
def pipe_escape(text: str) -> str:
    """Escape pipes in text for pipe format"""
    return str(text).replace('|', '\\|')

def format_output(data: Dict[str, Any], format_type: str = 'pipe') -> str:
    """Format output data according to specified format"""
    if format_type == 'json':
        return json.dumps(data)
    elif format_type == 'pipe':
        # Simple pipe format for single values
        if len(data) == 1:
            return pipe_escape(str(list(data.values())[0]))
        # Multiple values
        parts = [f"{k}:{pipe_escape(str(v))}" for k, v in data.items()]
        return '|'.join(parts)
    else:
        # Text format
        return ' | '.join(f"{k}: {v}" for k, v in data.items())

# ============= MCP SERVER HELPERS =============
def create_mcp_response(request_id: Any, result: Any = None, error: Any = None) -> Dict:
    """Create standard MCP JSON-RPC response"""
    response = {"jsonrpc": "2.0", "id": request_id}
    if error:
        response["error"] = error
    else:
        response["result"] = result
    return response

def _create_tool_response(content: str) -> Dict:
    """Create standard tool response format (internal)"""
    return {
        "content": [{
            "type": "text",
            "text": content
        }]
    }

# Keep public alias for backwards compatibility
def create_tool_response(content: str) -> Dict:
    """Create standard tool response format"""
    return _create_tool_response(content)

def send_response(response: Dict):
    """Send JSON-RPC response to stdout"""
    print(json.dumps(response), flush=True)

def create_server_info(name: str, version: str, description: str) -> Dict:
    """Create standard server info for initialization"""
    return {
        "protocolVersion": "2024-11-05",
        "capabilities": {"tools": {}},
        "serverInfo": {
            "name": name,
            "version": version,
            "description": description
        }
    }

def create_tool_schema(name: str, description: str, properties: Dict, required: list = None) -> Dict:
    """Create standard tool schema"""
    return {
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required or [],
            "additionalProperties": True
        }
    }

# ============= SERVER LOOP HELPER =============
class MCPServer:
    """Base MCP server class with standard loop handling"""
    
    def __init__(self, name: str, version: str, description: str):
        self.name = name
        self.version = version
        self.description = description
        self.logger = setup_logging(name)
        self.tools = {}
        self.running = True
    
    def register_tool(self, tool_func, name: str, description: str, 
                     properties: Dict, required: list = None):
        """Register a tool handler"""
        self.tools[name] = {
            'func': tool_func,
            'schema': create_tool_schema(name, description, properties, required)
        }
    
    def handle_initialize(self, params: Dict) -> Dict:
        """Handle initialization request"""
        return create_server_info(self.name, self.version, self.description)
    
    def handle_tools_list(self, params: Dict) -> Dict:
        """Handle tools list request"""
        return {
            "tools": [tool['schema'] for tool in self.tools.values()]
        }
    
    def handle_tools_call(self, params: Dict) -> Dict:
        """Handle tool call request"""
        tool_name = params.get("name", "")
        tool_args = params.get("arguments", {})
        
        if tool_name not in self.tools:
            return create_tool_response(f"Error: Unknown tool '{tool_name}'")
        
        try:
            result = self.tools[tool_name]['func'](**tool_args)
            # Format result as text
            if isinstance(result, dict):
                if "error" in result:
                    text = f"Error: {result['error']}"
                else:
                    # Tool-specific formatting
                    text = self.format_tool_result(tool_name, result)
            else:
                text = str(result)
            
            return create_tool_response(text)
        except Exception as e:
            self.logger.error(f"Tool error: {e}", exc_info=True)
            return create_tool_response(f"Error: {str(e)}")
    
    def format_tool_result(self, tool_name: str, result: Dict) -> str:
        """Format tool result - override in subclass for custom formatting"""
        # Default formatting
        if len(result) == 1:
            return str(list(result.values())[0])
        return json.dumps(result)
    
    def run(self):
        """Main server loop"""
        self.logger.info(f"{self.name} v{self.version} starting...")
        self.logger.info(f"Identity: {CURRENT_AI_ID}")
        
        while self.running:
            try:
                line = sys.stdin.readline()
                if not line:
                    break
                
                line = line.strip()
                if not line:
                    continue
                
                request = json.loads(line)
                request_id = request.get("id")
                method = request.get("method", "")
                params = request.get("params", {})
                
                # Route method
                if method == "initialize":
                    result = self.handle_initialize(params)
                    send_response(create_mcp_response(request_id, result))
                
                elif method == "notifications/initialized":
                    continue
                
                elif method == "tools/list":
                    result = self.handle_tools_list(params)
                    send_response(create_mcp_response(request_id, result))
                
                elif method == "tools/call":
                    result = self.handle_tools_call(params)
                    send_response(create_mcp_response(request_id, result))
                
                else:
                    send_response(create_mcp_response(request_id, {}))
            
            except KeyboardInterrupt:
                self.logger.info("Shutdown requested")
                break
            except Exception as e:
                self.logger.error(f"Server error: {e}", exc_info=True)
                if 'request_id' in locals():
                    send_response(create_mcp_response(
                        request_id, 
                        error={"code": -32603, "message": str(e)}
                    ))
        
        self.logger.info(f"{self.name} shutting down")

# ============= TIME UTILITIES =============
def format_time_compact(dt: datetime) -> str:
    """Format datetime compactly for display"""
    if not dt:
        return "unknown"
    
    if isinstance(dt, str):
        try:
            dt = datetime.fromisoformat(dt.replace('Z', '+00:00'))
        except:
            return dt[:10]
    
    now = datetime.now(timezone.utc)
    # Ensure dt is timezone-aware for comparison
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    delta = now - dt
    
    if delta.total_seconds() < 60:
        return "now"
    elif delta.total_seconds() < 3600:
        return f"{int(delta.total_seconds()/60)}m"
    elif dt.date() == now.date():
        return dt.strftime("%H:%M")
    elif delta.days == 1:
        return f"yesterday {dt.strftime('%H:%M')}"
    elif delta.days < 7:
        return f"{delta.days}d ago"
    else:
        return dt.strftime("%Y-%m-%d")

# ============= AI-FOCUSED RATE LIMITING =============
class AIRateLimiter:
    """
    Rate limiter designed for AI behavior patterns.
    Prevents: runaway loops, error cascades, infinite retries.
    NOT designed for human abuse prevention - these are AI-only tools.
    """

    def __init__(self, tool_name: str):
        self.tool_name = tool_name
        self.data_dir = get_tool_data_dir(tool_name)
        self.state_file = self.data_dir / ".rate_limit_state"

        # Limits tuned for AI usage
        self.max_calls_per_second = 10    # Prevent tight loops
        self.max_calls_per_minute = 100   # Prevent runaway operations
        self.max_errors_per_minute = 20   # Detect error cascades

        # State
        self.recent_calls = []    # [(timestamp, success)]
        self.load_state()

    def load_state(self):
        """Load rate limit state"""
        # SECURITY FIX: Avoid TOCTOU - try to open directly
        try:
            with open(self.state_file, 'r') as f:
                data = json.load(f)
                # Only keep recent data (last 5 minutes)
                cutoff = datetime.now().timestamp() - 300
                self.recent_calls = [(ts, success) for ts, success in data.get('calls', [])
                                    if ts > cutoff]
        except FileNotFoundError:
            self.recent_calls = []  # File doesn't exist yet
        except Exception:
            self.recent_calls = []  # Corrupted or other error

    def save_state(self):
        """Save rate limit state"""
        try:
            with open(self.state_file, 'w') as f:
                json.dump({'calls': self.recent_calls}, f)
        except:
            pass

    def check_and_record(self, success: bool = True) -> tuple[bool, Optional[str]]:
        """
        Check rate limits and record call.
        Returns: (allowed: bool, reason: Optional[str])
        """
        now = datetime.now().timestamp()

        # Clean old calls (older than 1 minute)
        cutoff_minute = now - 60
        cutoff_second = now - 1
        self.recent_calls = [(ts, s) for ts, s in self.recent_calls if ts > cutoff_minute]

        # Check per-second limit
        calls_last_second = sum(1 for ts, _ in self.recent_calls if ts > cutoff_second)
        if calls_last_second >= self.max_calls_per_second:
            return False, f"Rate limit: {self.max_calls_per_second} calls/sec (runaway loop?)"

        # Check per-minute limit
        calls_last_minute = len(self.recent_calls)
        if calls_last_minute >= self.max_calls_per_minute:
            return False, f"Rate limit: {self.max_calls_per_minute} calls/min (excessive usage)"

        # Check error rate
        errors_last_minute = sum(1 for _, s in self.recent_calls if not s)
        if errors_last_minute >= self.max_errors_per_minute:
            return False, f"Error cascade detected: {errors_last_minute} errors/min"

        # Record this call
        self.recent_calls.append((now, success))
        self.save_state()

        return True, None

    def get_stats(self) -> Dict:
        """Get current rate limit statistics"""
        now = datetime.now().timestamp()
        cutoff_minute = now - 60
        cutoff_second = now - 1

        recent = [(ts, s) for ts, s in self.recent_calls if ts > cutoff_minute]
        calls_per_second = sum(1 for ts, _ in recent if ts > cutoff_second)
        calls_per_minute = len(recent)
        errors_per_minute = sum(1 for _, s in recent if not s)

        return {
            'calls_per_second': calls_per_second,
            'calls_per_minute': calls_per_minute,
            'errors_per_minute': errors_per_minute,
            'limits': {
                'max_per_second': self.max_calls_per_second,
                'max_per_minute': self.max_calls_per_minute,
                'max_errors': self.max_errors_per_minute
            }
        }

# ============= OPERATION TRACKING =============
class OperationTracker:
    """Track last operation for tool chaining"""

    def __init__(self, tool_name: str):
        self.tool_name = tool_name
        self.data_dir = get_tool_data_dir(tool_name)
        self.op_file = self.data_dir / ".last_operation"
        self.last_op = None

    def save(self, op_type: str, result: Any):
        """Save operation"""
        self.last_op = {
            'type': op_type,
            'result': result,
            'time': datetime.now()
        }
        try:
            with open(self.op_file, 'w') as f:
                json.dump({
                    'type': op_type,
                    'time': self.last_op['time'].isoformat()
                }, f)
        except:
            pass

    def get(self) -> Optional[Dict]:
        """Get last operation"""
        if self.last_op:
            return self.last_op

        # SECURITY FIX: Avoid TOCTOU - try to open directly
        try:
            with open(self.op_file, 'r') as f:
                data = json.load(f)
                return {
                    'type': data['type'],
                    'time': datetime.fromisoformat(data['time'])
                }
        except FileNotFoundError:
            return None  # File doesn't exist
        except Exception:
            return None  # Corrupted or other error
