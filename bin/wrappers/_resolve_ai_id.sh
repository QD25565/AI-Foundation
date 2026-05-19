#!/bin/sh
# _resolve_ai_id.sh — shared AI_ID resolver for teambook/notebook wrappers
# Source this file; it exports AI_ID if not already set.
# Never called directly.

_resolve_ai_id() {
    # Fast path: already set in environment
    if [ -n "${AI_ID:-}" ]; then
        export AI_ID
        export WSLENV="${WSLENV:+$WSLENV:}AI_ID"
        return 0
    fi

    # Strategy 1: walk up from CWD looking for .claude/settings.json
    _dir="$PWD"
    while true; do
        _settings="$_dir/.claude/settings.json"
        if [ -f "$_settings" ]; then
            _found=$(python3 -c "
import json, sys
try:
    d = json.load(open('$_settings'))
    v = d.get('env', {}).get('AI_ID', '')
    if v:
        print(v)
except Exception:
    pass
" 2>/dev/null)
            if [ -n "$_found" ]; then
                export AI_ID="$_found"
                export WSLENV="${WSLENV:+$WSLENV:}AI_ID"
                return 0
            fi
        fi
        # Stop at filesystem root
        if [ "$_dir" = "/" ]; then
            break
        fi
        _dir="${_dir%/*}"
        if [ -z "$_dir" ]; then
            _dir="/"
        fi
    done

    # Strategy 2: match CWD against instances.toml (Windows paths → /mnt/c/...)
    _toml="$HOME/.ai-foundation/instances.toml"
    if [ -f "$_toml" ]; then
        _found=$(python3 -c "
import sys, re

cwd = '$PWD'
toml_path = '$_toml'

# Normalize CWD: /mnt/c/Users/... -> C:/Users/... (forward slashes)
# so we can compare against the Windows paths in instances.toml
def to_win_fwd(p):
    import re
    m = re.match(r'^/mnt/([a-zA-Z])(/.*)?\$', p)
    if m:
        drive = m.group(1).upper()
        rest  = (m.group(2) or '').replace('/', '/')
        return drive + ':' + rest
    return p

cwd_win = to_win_fwd(cwd)

try:
    with open(toml_path) as f:
        content = f.read()

    # Parse [[instances]] blocks with path + ai_id
    blocks = re.split(r'\[\[instances\]\]', content)[1:]
    best_ai_id = ''
    best_len   = 0
    for block in blocks:
        pm = re.search(r'path\s*=\s*[\"\'](.*?)[\"\']', block)
        im = re.search(r'ai_id\s*=\s*[\"\'](.*?)[\"\']', block)
        if not pm or not im:
            continue
        inst_path = pm.group(1).replace('\\\\', '/').replace('\\', '/')
        ai_id     = im.group(1)
        if cwd_win.startswith(inst_path) and len(inst_path) > best_len:
            best_len   = len(inst_path)
            best_ai_id = ai_id
    if best_ai_id:
        print(best_ai_id)
except Exception as e:
    pass
" 2>/dev/null)
        if [ -n "$_found" ]; then
            export AI_ID="$_found"
            export WSLENV="${WSLENV:+$WSLENV:}AI_ID"
            return 0
        fi
    fi

    # Strategy 3: check ~/.claude/settings.json (global fallback)
    _global="$HOME/.claude/settings.json"
    if [ -f "$_global" ]; then
        _found=$(python3 -c "
import json
try:
    d = json.load(open('$_global'))
    v = d.get('env', {}).get('AI_ID', '')
    if v:
        print(v)
except Exception:
    pass
" 2>/dev/null)
        if [ -n "$_found" ]; then
            export AI_ID="$_found"
            export WSLENV="${WSLENV:+$WSLENV:}AI_ID"
            return 0
        fi
    fi

    # AI_ID remains unset — the binary will use daemon-resolved identity
    return 0
}

_resolve_ai_id
