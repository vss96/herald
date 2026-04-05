# herald: Full Design Spec

> **Approach**: Direct tmux Orchestrator (Approach A, refined)
> **Stack**: Rust + ratatui + crossterm
> **Date**: 2026-04-05

## 1. Overview

A TUI that orchestrates multiple concurrent Claude Code sessions. One screen: main terminal area on the left, session sidebar on the right. Sessions that need human attention surface automatically via a priority queue.

```
┌──────────────────────────────────┬──────────────────┐
│                                  │  Sessions        │
│  Main Area                       │                  │
│  (live tmux pane via ctrl mode)  │  ● fix-tests     │
│                                  │    [NEEDS INPUT] │
│  $ claude                        │                  │
│  > Allow edit to foo.rs?         │  ◐ refactor      │
│  [y/n] _                         │    [running]     │
│                                  │                  │
│                                  │  ✓ add-auth      │
│                                  │    [done]        │
│                                  │                  │
├──────────────────────────────────┤  ○ new-feature   │
│ [n]ew  [k]ill  [r]ename  [j/k]  │    [queued]      │
└──────────────────────────────────┴──────────────────┘
```

## 2. Architecture

### 2.1 Component Overview

```
┌─────────────────────────────────────────────────────────────┐
│  herald (single Rust binary)                            │
│                                                             │
│  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌───────────┐  │
│  │ TUI      │  │ Session   │  │ Event    │  │ Priority  │  │
│  │ Renderer │◄─┤ Manager   │◄─┤ Listener │◄─┤ Queue     │  │
│  │ (ratatui)│  │           │  │          │  │           │  │
│  └──────────┘  └─────┬─────┘  └────┬─────┘  └───────────┘  │
│                      │              │                        │
│              tmux control mode   per-session                 │
│              (-CC subprocess)    Unix sockets                │
└──────────────┬──────────────────────┬───────────────────────┘
               │                      │
               ▼                      ▼
┌──────────────────────┐  ┌───────────────────────────────┐
│  tmux server         │  │  /tmp/herald/<sid>.sock   │
│                      │  │  (one socket per session)     │
│  window:0 → claude   │  └───────────────────────────────┘
│  window:1 → claude   │         ▲
│  window:2 → claude   │         │ hooks write JSON
└──────────────────────┘         │
         ▲                ┌──────┴──────────────────┐
         │                │  Claude Code instances   │
         └────────────────┤  (running inside tmux)   │
                          └──────────────────────────┘
```

### 2.2 tmux Control Mode (replacing capture-pane)

Instead of polling `tmux capture-pane`, we use **tmux control mode** (`tmux -CC`) as a subprocess. This gives us:

- **`%output %<pane-id> <data>`** events streamed in real-time for every pane
- No polling, no snapshot staleness, no cursor position loss
- Flow control via `refresh-client -f pause-after=<seconds>` to handle fast output
- Resize handling via `refresh-client -C <cols>x<rows>`

The TUI maintains a **virtual terminal buffer** per session using the `vte` crate to parse ANSI escape sequences from the `%output` stream. The active session's buffer is rendered in the main area.

**Input path**: Keyboard input → `send-keys -t %<pane-id> <key>` → tmux → Claude Code

### 2.3 Per-Session Unix Sockets (replacing shared socket)

Each session gets its own socket in a **private, per-user runtime directory**:

```
$XDG_RUNTIME_DIR/herald/<session-id>.sock   (Linux, typically /run/user/<uid>/herald/)
$TMPDIR/herald-$UID/<session-id>.sock        (macOS fallback)
```

**Security model:**
- Runtime directory created with `mkdir -m 0700` — only the owning user can list or access contents
- Sockets created with umask 077 → mode 0600
- Buffer files created with mode 0600
- On startup, verify the runtime directory is owned by the current user and has mode 0700; refuse to start if not
- Payloads in buffer files are redacted: `tool_input` is stored as `{"redacted": true}` — only `hook_event_name`, `session_id`, `tool_name`, and `tool_use_id` are persisted

**Properties:**
- **No interleaving**: Events are inherently scoped to one session
- **Fault isolation**: One dead socket doesn't affect others
- **Startup discovery**: On TUI restart, enumerate `<runtime-dir>/herald/*.sock` to find surviving sessions

The TUI spawns an async listener (tokio) per socket. Events are deserialized and pushed to the priority queue.

### 2.4 Hook Configuration

Claude Code hooks are configured per-session at launch time. The hook script is a small shell script bundled with herald that writes JSON to the session's socket.

**Events we hook into:**

| Hook Event | What We Detect | Priority |
|---|---|---|
| `PermissionRequest` | User approval needed for a tool call | HIGH |
| `PostToolUseFailure` | Tool error occurred | CRITICAL |
| `PreToolUse` (matcher: `""`) | Tool activity (sidebar status updates) | INFO (sidebar only) |
| `PostToolUse` | Tool completed (clears pending permission state) | INFO (internal) |
| `Stop` | Session finished or stopped | LOW |
| `Notification` | Status updates, progress | INFO (sidebar only) |
| `SessionStart` | Session initialized | INFO (sidebar only) |
| `SessionEnd` | Session terminated | INFO (sidebar only) |

> **Why `PermissionRequest` instead of `PreToolUse`?** `PreToolUse` fires for *every* tool call, not just ones needing approval. Using it as the permission signal would cause constant false-positive queue entries. `PermissionRequest` fires only when Claude Code is actually waiting for user input.

**Hook script** (`herald-hook.sh`):
```bash
#!/bin/bash
# Reads JSON from stdin, buffers redacted event to file, then delivers full event to socket
SESSION_ID="$CLAUDE_SESSION_ID"
RUNTIME_DIR="${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}/herald-$(id -u)}/herald"
SOCKET="${RUNTIME_DIR}/${SESSION_ID}.sock"
BUFFER="${RUNTIME_DIR}/${SESSION_ID}.buffer"
EVENT=$(cat)

# Redact tool_input for buffer persistence (keep only routing fields)
REDACTED=$(echo "$EVENT" | jq -c '{session_id, hook_event_name, tool_name, tool_use_id}')

# Append redacted event to bounded buffer file
echo "$REDACTED" >> "$BUFFER"

# Enforce buffer size limit: keep last 500 lines
if [ "$(wc -l < "$BUFFER")" -gt 500 ]; then
  tail -n 500 "$BUFFER" > "${BUFFER}.tmp" && mv "${BUFFER}.tmp" "$BUFFER"
fi

# Attempt full event delivery via socket; if it fails, redacted event persists in buffer
echo "$EVENT" | socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null
```

> **Buffered delivery with bounds**: Events are always written (redacted) to a per-session buffer file before socket delivery. Buffer files are capped at 500 events — oldest are rotated out. If the TUI is down, events accumulate in the buffer. On startup/reconnect, the TUI drains the buffer file, then truncates it. Full `tool_input` is only sent via the live socket, never persisted to disk.

**Hook payload** (received from Claude Code on stdin):
```json
{
  "session_id": "abc123",
  "hook_event_name": "PermissionRequest",
  "tool_name": "Edit",
  "tool_use_id": "toolu_01ABC...",
  "tool_input": { "file_path": "/src/foo.rs", ... },
  "cwd": "/home/user/project"
}
```

> **Correlation key**: `tool_use_id` is present in `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, and `PermissionRequest` payloads. It uniquely identifies a single tool invocation, allowing us to match a `PermissionRequest` to its resolving `PostToolUse`. If `tool_use_id` is absent (defensive), fall back to matching by `session_id` + `tool_name` with a 30-second expiry window.

## 3. Priority Queue

### 3.1 Priority Levels

```
CRITICAL  → PostToolUseFailure (tool errors, crashes)
HIGH      → PermissionRequest (user approval needed)
LOW       → Stop (session completed, review results)
INFO      → Sidebar-only updates (no queue entry)
```

### 3.2 Queue Semantics

- **Per-session state machine**: Each session has exactly one queue entry at most. New events for a session *replace* its existing entry if higher priority, otherwise ignored.
- **Debounce**: Repeated errors from the same session within 2 seconds are coalesced (only the latest is shown).
- **Aging**: After 30 seconds in the queue without being addressed, a session's entry gets a visual indicator (blinking/highlight) but does NOT auto-promote in priority.
- **Resolution-based clearing**: Queue entries are NOT cleared by focusing a session. They persist until the underlying condition is resolved:
  - Permission prompts: cleared when `PostToolUse` fires for the same `tool_use_id` (meaning the user responded). Fallback: if no `tool_use_id` match within 30 seconds, clear on any `PostToolUse` from the same session.
  - Errors: cleared when the user explicitly dismisses (keybind) or a new non-error event arrives from the same session
  - Completions: cleared when the user focuses the session and presses a dismiss key
  - Safety valve: any queue entry older than 5 minutes without resolution gets a "stale" indicator and can be manually dismissed
- **Fairness**: Within the same priority tier, FIFO ordering. A session cannot re-enter the same tier faster than once per 5 seconds (prevents thrashing from a noisy session).

### 3.3 Main Area Behavior

- When the queue is empty: main area shows the last-selected session (or a welcome/new-session screen)
- When an item enters the queue: if the user is idle (no keypress for 3 seconds), auto-switch to the highest-priority session. If the user is actively typing, show a non-intrusive notification in the sidebar instead.
- User can always manually select any session from the sidebar, regardless of queue state.

## 4. Session Manager

### 4.1 Session Lifecycle

```
Created → Starting → Running → [NeedsAttention] → Running → ... → Stopped
                                      ↑                              │
                                      └──── (new event) ◄───────────┘
```

### 4.2 Session Data Model

```rust
struct Session {
    id: String,                    // UUID
    nickname: String,              // User-assigned or auto-generated
    tmux_pane_id: String,          // e.g., "%0"
    prompt: String,                // Original prompt
    working_dir: PathBuf,          // Project directory
    status: SessionStatus,         // Running, NeedsAttention, Stopped, Error
    created_at: Instant,
    terminal_buffer: TerminalBuffer, // vte-parsed screen state
}

enum SessionStatus {
    Starting,
    Running { last_activity: Instant },
    NeedsAttention { reason: AttentionReason, since: Instant },
    Stopped { exit_code: Option<i32> },
    Error { message: String },
}

enum AttentionReason {
    PermissionPrompt { tool_name: String },
    ToolError { tool_name: String, error: String },
    Completed,
}
```

### 4.3 Launching a Session

1. Generate session ID (UUID)
2. Ensure runtime directory exists (`<runtime-dir>/herald/`, mode 0700)
3. Create per-session socket at `<runtime-dir>/herald/<session-id>.sock` (mode 0600)
4. Create per-session buffer file at `<runtime-dir>/herald/<session-id>.buffer` (mode 0600)
4. Start async socket listener
5. Create tmux window: `tmux new-window -t herald -n <nickname>`
6. Store session ID in tmux pane metadata: `tmux set-option -p -t %<pane-id> @claude_ext_session_id "<uuid>"`
7. Configure hooks by writing a temporary `.claude/settings.local.json` in the working directory (or use `--hooks` CLI flag if available)
8. Start Claude Code in the tmux window: `claude -p "<prompt>"` (or interactive mode)
9. Register the pane ID from tmux and start receiving `%output` events

### 4.4 Durable Session Identity

Each tmux pane stores its session ID in tmux user metadata:
```bash
tmux set-option -p -t %<pane-id> @claude_ext_session_id "<uuid>"
```

This survives TUI restarts — the session ID is stored by tmux itself, not by our process.

### 4.5 Resync on TUI Restart

On startup, herald:
1. Check if tmux session `herald` exists (`tmux has-session -t herald`)
2. If yes, enumerate panes with metadata: `tmux list-panes -s -t herald -F "#{pane_id} #{window_name} #{@claude_ext_session_id}"`
3. For each pane with a valid `@claude_ext_session_id`:
   a. Locate matching socket at `/tmp/herald/<session-id>.sock`
   b. Drain the buffer file `/tmp/herald/<session-id>.buffer` for any missed events
   c. Rebuild session state from buffered events
4. Re-attach control mode client to resume `%output` streaming
5. For any pane without metadata → treat as orphan, show in sidebar as "unknown session" with option to adopt or kill
6. For any socket without a matching pane → clean up stale socket and buffer files

## 5. TUI Layout

### 5.1 Key Bindings

| Key | Action |
|---|---|
| `n` | New session (opens prompt input) |
| `k` | Kill selected session |
| `r` | Rename selected session |
| `j` / `↓` | Next session in sidebar |
| `k` / `↑` | Previous session in sidebar |
| `Enter` | Focus selected session in main area |
| `Tab` | Toggle sidebar focus vs main area focus |
| `Ctrl-n` | Jump to next queued (needs-attention) session |
| `q` | Quit TUI (sessions persist in tmux) |

When main area is focused, all keys are forwarded to the tmux pane (Claude Code) except `Esc` which returns focus to the sidebar.

### 5.2 Sidebar Session Display

```
  Sessions (3 active)
  ─────────────────────
  ● fix-tests          
    [PERMISSION] 12s   ← needs attention, timer
                       
  ◐ refactor           
    [running] Edit...  ← last tool name
                       
  ✓ add-auth           
    [done] 2m ago      ← completed, time
```

Status indicators:
- `●` Red: needs attention (error or permission)
- `◐` Yellow/animated: running
- `✓` Green: completed
- `○` Gray: queued/starting

### 5.3 New Session Dialog

Inline prompt at the bottom of the screen:
```
┌─ New Session ────────────────────────────────┐
│ Nickname: fix-tests                          │
│ Directory: /Users/vikas/project (default)    │
│ Prompt: fix the failing tests in auth module │
│                                              │
│ [Enter] Launch   [Esc] Cancel                │
└──────────────────────────────────────────────┘
```

## 6. Rust Crate Dependencies

| Crate | Purpose |
|---|---|
| `ratatui` | TUI framework |
| `crossterm` | Terminal backend |
| `tokio` | Async runtime (socket listeners, tmux IO) |
| `vte` | ANSI/VT100 escape sequence parser |
| `serde` / `serde_json` | JSON deserialization for hook events |
| `uuid` | Session ID generation |
| `dirs` | Platform paths |

## 7. File Structure

```
herald/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point, tokio runtime
│   ├── app.rs               # App state, event loop
│   ├── tui/
│   │   ├── mod.rs
│   │   ├── layout.rs        # Main area + sidebar layout
│   │   ├── sidebar.rs       # Session list widget
│   │   ├── main_area.rs     # Terminal buffer renderer
│   │   └── dialogs.rs       # New session, rename, etc.
│   ├── session/
│   │   ├── mod.rs
│   │   ├── manager.rs       # Session lifecycle, tmux commands
│   │   ├── model.rs         # Session, SessionStatus, etc.
│   │   └── terminal.rs      # vte-based terminal buffer
│   ├── events/
│   │   ├── mod.rs
│   │   ├── hook_listener.rs # Per-session socket listener
│   │   ├── queue.rs         # Priority queue with fairness
│   │   └── types.rs         # HookEvent, AttentionReason
│   └── tmux/
│       ├── mod.rs
│       ├── control.rs       # Control mode client (-CC)
│       └── commands.rs      # tmux CLI wrappers
├── scripts/
│   └── herald-hook.sh   # Hook script written to /tmp at runtime
└── docs/
    ├── problem-statement.md
    ├── design-approaches.md
    └── spec.md              # This file
```

## 8. Verification Plan

### 8.1 Manual Testing

1. **Single session**: Launch herald, create one session, verify terminal output streams in real-time, input forwarding works (type a response, see it appear)
2. **Permission surfacing**: Start a session that will trigger a permission prompt. Verify it appears in the queue and auto-switches to main area.
3. **Multi-session**: Run 3 sessions simultaneously. Verify sidebar updates, switching between sessions preserves terminal state.
4. **TUI restart**: Kill the TUI (`q`), relaunch. Verify sessions are rediscovered and terminal buffers repopulate.
5. **Noisy session**: Create a session that rapidly emits errors. Verify debounce works and other sessions aren't starved.

### 8.2 Integration Tests

- Spawn tmux control mode, send known output, verify vte parsing produces correct screen buffer
- Write events to a Unix socket, verify priority queue ordering and fairness rules
- Test session lifecycle state machine transitions

### 8.3 Edge Cases

- tmux not installed → clear error message on startup
- Socket directory doesn't exist → create `/tmp/herald/` on startup
- Session exits while in main area → update status, don't crash
- Terminal resize → propagate to tmux via `refresh-client -C`
- Very long-running session → no memory leaks in terminal buffer (cap scrollback)
