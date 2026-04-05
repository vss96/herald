#!/bin/bash
# Herald hook script — receives Claude Code hook events on stdin,
# buffers a redacted copy to file, delivers full event to socket.
#
# Environment variables (set by herald when configuring hooks):
#   CLAUDE_SESSION_ID  — session UUID
#   HERALD_SOCKET      — path to per-session Unix socket

set -euo pipefail

SESSION_ID="${CLAUDE_SESSION_ID:-unknown}"
SOCKET="${HERALD_SOCKET:-}"
RUNTIME_DIR="$(dirname "$SOCKET")"
BUFFER="${RUNTIME_DIR}/${SESSION_ID}.buffer"

# Read full event from stdin
EVENT=$(cat)

# Redact tool_input for buffer persistence — NEVER write unredacted events to disk.
# Try jq first, then python as fallback, then a conservative grep-based strip.
redact_event() {
    if command -v jq &>/dev/null; then
        echo "$EVENT" | jq -c '{session_id, hook_event_name, tool_name, tool_use_id}' 2>/dev/null && return
    fi
    if command -v python3 &>/dev/null; then
        echo "$EVENT" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
print(json.dumps({k: d.get(k) for k in ('session_id','hook_event_name','tool_name','tool_use_id')}))" 2>/dev/null && return
    fi
    # Last resort: extract only the hook_event_name and session_id with grep
    # This is lossy but safe — never leaks tool_input
    local name=$(echo "$EVENT" | grep -o '"hook_event_name":"[^"]*"' | head -1)
    local sid=$(echo "$EVENT" | grep -o '"session_id":"[^"]*"' | head -1)
    echo "{${sid},${name}}"
}

REDACTED=$(redact_event)

# Append redacted event to bounded buffer file.
# Use a lockfile with mkdir (atomic, works on macOS and Linux — no flock needed).
LOCKDIR="${BUFFER}.lock"
lock_acquired=false
for i in 1 2 3 4 5; do
    if mkdir "$LOCKDIR" 2>/dev/null; then
        lock_acquired=true
        break
    fi
    # Brief sleep before retry (stale lock cleanup after 10s)
    if [ -d "$LOCKDIR" ]; then
        lock_age=$(( $(date +%s) - $(stat -f %m "$LOCKDIR" 2>/dev/null || stat -c %Y "$LOCKDIR" 2>/dev/null || echo 0) ))
        if [ "$lock_age" -gt 10 ]; then
            rmdir "$LOCKDIR" 2>/dev/null || true
        fi
    fi
    sleep 0.1
done

if [ "$lock_acquired" = true ]; then
    echo "$REDACTED" >> "$BUFFER" 2>/dev/null || true

    # Enforce buffer size limit: keep last 500 lines
    if [ -f "$BUFFER" ]; then
        LINE_COUNT=$(wc -l < "$BUFFER" 2>/dev/null || echo 0)
        if [ "$LINE_COUNT" -gt 500 ]; then
            tail -n 500 "$BUFFER" > "${BUFFER}.tmp" && mv "${BUFFER}.tmp" "$BUFFER"
        fi
    fi

    rmdir "$LOCKDIR" 2>/dev/null || true
else
    # Couldn't acquire lock — write anyway (better than losing the event)
    echo "$REDACTED" >> "$BUFFER" 2>/dev/null || true
fi

# Attempt full event delivery via socket.
# Try socat first, then python3 (always available on macOS), then nc.
deliver_to_socket() {
    if [ -z "$SOCKET" ] || [ ! -S "$SOCKET" ]; then
        return 1
    fi

    if command -v socat &>/dev/null; then
        echo "$EVENT" | socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null && return 0
    fi

    if command -v python3 &>/dev/null; then
        python3 -c "
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
try:
    s.connect('$SOCKET')
    s.sendall(sys.stdin.buffer.read())
    s.sendall(b'\n')
    s.close()
except:
    sys.exit(1)
" <<< "$EVENT" 2>/dev/null && return 0
    fi

    if command -v nc &>/dev/null; then
        echo "$EVENT" | nc -U "$SOCKET" 2>/dev/null && return 0
    fi

    return 1
}

deliver_to_socket || true
