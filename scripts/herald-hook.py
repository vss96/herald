#!/usr/bin/env python3
"""Herald hook script — receives Claude Code hook events on stdin,
buffers a redacted copy to file, delivers full event to Unix socket.

Environment variables (set by herald when configuring hooks):
    CLAUDE_SESSION_ID  — session UUID
    HERALD_SOCKET      — path to per-session Unix socket
"""

import json
import os
import socket
import sys
import fcntl

def main():
    session_id = os.environ.get("CLAUDE_SESSION_ID", "unknown")
    socket_path = os.environ.get("HERALD_SOCKET", "")
    if not socket_path:
        return

    runtime_dir = os.path.dirname(socket_path)
    buffer_path = os.path.join(runtime_dir, f"{session_id}.buffer")

    # Read full event from stdin
    event_raw = sys.stdin.read().strip()
    if not event_raw:
        return

    # Redact: keep only routing fields, never persist tool_input
    try:
        event = json.loads(event_raw)
        redacted = json.dumps({
            "session_id": event.get("session_id"),
            "hook_event_name": event.get("hook_event_name"),
            "tool_name": event.get("tool_name"),
            "tool_use_id": event.get("tool_use_id"),
        })
    except (json.JSONDecodeError, TypeError):
        redacted = event_raw

    # Append redacted event to bounded buffer file with file locking
    try:
        with open(buffer_path, "a") as f:
            fcntl.flock(f.fileno(), fcntl.LOCK_EX)
            f.write(redacted + "\n")

            # Enforce buffer size limit: keep last 500 lines
            f.flush()
            fcntl.flock(f.fileno(), fcntl.LOCK_UN)

        # Check line count and truncate if needed
        try:
            with open(buffer_path, "r") as f:
                lines = f.readlines()
            if len(lines) > 500:
                with open(buffer_path, "w") as f:
                    fcntl.flock(f.fileno(), fcntl.LOCK_EX)
                    f.writelines(lines[-500:])
                    fcntl.flock(f.fileno(), fcntl.LOCK_UN)
        except OSError:
            pass
    except OSError:
        pass

    # Deliver full event to Unix socket
    if socket_path and os.path.exists(socket_path):
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.settimeout(2)
            s.connect(socket_path)
            s.sendall((event_raw + "\n").encode())
            s.close()
        except (OSError, socket.error):
            pass  # Event is in the buffer file as fallback

if __name__ == "__main__":
    main()
