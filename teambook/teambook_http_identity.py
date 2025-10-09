#!/usr/bin/env python3
"""HTTP identity server for Teambook deployments."""

import argparse
import json
import logging
import os
import re
from datetime import datetime, timezone
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, Optional, List
from urllib.parse import parse_qs, urlparse

from teambook_shared import (
    CURRENT_AI_DISPLAY_NAME,
    CURRENT_AI_FINGERPRINT,
    CURRENT_AI_ID,
    CURRENT_AI_PROTOCOL_HANDLES,
    CURRENT_IDENTITY_HANDLES,
    CURRENT_AI_PUBLIC_KEY,
    build_security_envelope,
    get_current_ai_identity,
    resolve_current_handle,
)


def _parse_bool(value: Optional[str], default: bool = False) -> bool:
    if value is None:
        return default
    return str(value).strip().lower() not in {"0", "false", "no", "off", ""}


def _parse_int(value: Optional[str]) -> Optional[int]:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _capabilities_from_query(query: Dict[str, Any]) -> Dict[str, Any]:
    capabilities: Dict[str, Any] = {}

    pattern = query.get("pattern")
    if pattern:
        capabilities["pattern"] = pattern

    max_length = _parse_int(query.get("max_length"))
    if max_length:
        capabilities["max_length"] = max_length

    for key in ("supports_spaces", "supports_unicode", "prefer_ascii"):
        if key in query:
            capabilities[key] = _parse_bool(query.get(key))

    return capabilities


def _identity_snapshot(
    protocol: Optional[str] = None,
    *,
    prefer_pretty: bool = False,
    capabilities: Optional[Dict[str, Any]] = None,
    protocols: Optional[List[str]] = None,
) -> Dict[str, Any]:
    identity = get_current_ai_identity()
    handles = identity.get("protocol_handles") or CURRENT_AI_PROTOCOL_HANDLES

    resolved_handle = resolve_current_handle(
        protocol,
        prefer_pretty=prefer_pretty,
        capabilities=capabilities,
        fallback=True,
    )

    resolved_handles = dict(CURRENT_IDENTITY_HANDLES)
    if protocols:
        for proto in protocols:
            proto_normalized = (proto or "").strip().lower()
            if not proto_normalized:
                continue
            resolved_handles[proto_normalized] = resolve_current_handle(
                proto_normalized,
                prefer_pretty=proto_normalized in {"cli", "terminal", "shell"},
                capabilities=capabilities,
                fallback=True,
            )
    resolved_handles = {key: value for key, value in resolved_handles.items() if value}

    base_payload = {
        "ai_id": identity.get("ai_id", CURRENT_AI_ID),
        "display_name": identity.get("display_name", CURRENT_AI_DISPLAY_NAME),
        "fingerprint": identity.get("fingerprint", CURRENT_AI_FINGERPRINT),
        "public_key": identity.get("public_key", CURRENT_AI_PUBLIC_KEY),
        "metadata_version": identity.get("version"),
        "handles": handles,
        "resolved_handle": resolved_handle,
        "resolved_handles": resolved_handles,
        "resolved_context": {
            "protocol": protocol,
            "prefer_pretty": prefer_pretty,
            "capabilities": capabilities or {},
        },
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }

    envelope = build_security_envelope(base_payload, "teambook.http.identity")

    pattern = (capabilities or {}).get("pattern")
    matches_pattern = True
    if isinstance(pattern, str) and pattern:
        try:
            matches_pattern = re.match(pattern, resolved_handle) is not None
        except re.error:
            matches_pattern = False

    return {
        "identity": base_payload,
        "envelope": envelope,
        "matches_pattern": matches_pattern,
    }


class IdentityServer(ThreadingHTTPServer):
    """Threaded HTTP server with context defaults."""

    def __init__(self, server_address, RequestHandlerClass, *, protocol=None, prefer_pretty=False):
        super().__init__(server_address, RequestHandlerClass)
        self.default_protocol = protocol
        self.default_pretty = prefer_pretty


class IdentityRequestHandler(BaseHTTPRequestHandler):
    server_version = "TeambookHTTPIdentity/1.0"

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A003
        logging.getLogger("teambook.http.identity").debug("%s - %s", self.address_string(), format % args)

    def _write_json(self, payload: Dict[str, Any], status: HTTPStatus = HTTPStatus.OK) -> None:
        body = json.dumps(payload, indent=2, sort_keys=True).encode("utf-8")
        self.send_response(status.value)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_OPTIONS(self) -> None:  # noqa: N802 (method name required by BaseHTTPRequestHandler)
        self.send_response(HTTPStatus.NO_CONTENT.value)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path.rstrip("/") or "/"

        if path in {"/", "/identity"}:
            query = {key: values[0] for key, values in parse_qs(parsed.query or "").items() if values}
            protocol = query.get("protocol", getattr(self.server, "default_protocol", None))
            prefer_pretty = _parse_bool(query.get("prefer_pretty"), getattr(self.server, "default_pretty", False))
            capabilities = _capabilities_from_query(query)
            requested_protocols = query.get("protocols")
            protocol_list = None
            if requested_protocols:
                protocol_list = [part.strip() for part in requested_protocols.split(",") if part.strip()]

            snapshot = _identity_snapshot(
                protocol,
                prefer_pretty=prefer_pretty,
                capabilities=capabilities,
                protocols=protocol_list,
            )
            snapshot["request"] = {
                "protocol": protocol,
                "prefer_pretty": prefer_pretty,
                "capabilities": capabilities,
                "protocols": protocol_list or [],
            }
            self._write_json(snapshot)
            return

        if path == "/health":
            self._write_json({"status": "ok", "ai_id": CURRENT_AI_ID})
            return

        self.send_error(HTTPStatus.NOT_FOUND.value, "Not Found")


def serve_http_identity(host: str, port: int, protocol: Optional[str], prefer_pretty: bool) -> None:
    server = IdentityServer((host, port), IdentityRequestHandler, protocol=protocol, prefer_pretty=prefer_pretty)
    logging.getLogger("teambook.http.identity").info("Identity server listening on %s:%s", host, port)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        logging.getLogger("teambook.http.identity").info("Identity server shutdown requested")
    finally:
        server.server_close()


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Start the Teambook HTTP identity service.")
    parser.add_argument("--host", default=os.environ.get("TEAMBOOK_HTTP_HOST", "0.0.0.0"), help="Host interface to bind.")
    parser.add_argument(
        "--port",
        type=int,
        default=int(os.environ.get("TEAMBOOK_HTTP_PORT", "8130")),
        help="Port to listen on (default: 8130).",
    )
    parser.add_argument("--protocol", default=os.environ.get("TEAMBOOK_HTTP_PROTOCOL"), help="Default protocol hint.")
    parser.add_argument(
        "--prefer-pretty",
        action="store_true",
        default=_parse_bool(os.environ.get("TEAMBOOK_HTTP_PREFER_PRETTY")),
        help="Prefer pretty handles for defaults.",
    )
    parser.add_argument(
        "--once",
        action="store_true",
        help="Emit one identity snapshot to stdout and exit.",
    )
    parser.add_argument(
        "--log-level",
        default=os.environ.get("TEAMBOOK_HTTP_LOG_LEVEL", "INFO"),
        help="Logging level (default: INFO).",
    )

    args = parser.parse_args(argv)

    logging.basicConfig(level=getattr(logging, str(args.log_level).upper(), logging.INFO))

    if args.once:
        snapshot = _identity_snapshot(args.protocol, prefer_pretty=args.prefer_pretty)
        print(json.dumps(snapshot, indent=2, sort_keys=True))
        return 0

    serve_http_identity(args.host, args.port, args.protocol, args.prefer_pretty)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
