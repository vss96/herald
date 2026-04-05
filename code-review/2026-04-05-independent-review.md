# Herald — Independent Code Review

> **Date:** 2026-04-05
> **Scope:** Full review of all ~3,300 lines across 14 Rust source files
> **Reviewers:** 3 parallel review agents (core logic, event system, tmux+TUI)

## Summary

The architecture is sound — per-session Unix sockets, tmux control mode, priority queue with fairness/debounce, VTE terminal buffer, ratatui TUI. The core design decisions (Approach A from the design doc) are well-chosen.

**However, the async→sync bridge was never built, so the central feature (attention surfacing) doesn't function at runtime.**

---

## Critical (4)

### C1. Hook listeners never wired up

**Files:** `src/main.rs`, `src/session/manager.rs`

`HookListener::listen()` is never called. Sockets are never bound. The `mpsc` channel only exists in tests. At runtime: Claude Code hook events hit a refused socket, `handle_hook_event` is never called, the attention queue stays empty, auto-switch never triggers, sessions stay `Starting` forever.

**Fix:** In `SessionManager::launch`, create a `HookListener`, spawn a tokio task calling `listener.listen(tx)`. In `run_loop`, drain the `Receiver` each 50ms tick and call `app.handle_hook_event()`.

---

### C2. Shell injection via `working_dir` and `prompt`

**Files:** `src/tmux/commands.rs:105`, `src/session/manager.rs:58`

`working_dir.display()` is interpolated unquoted into a shell command passed to `send-keys`. `escaped_prompt` only escapes `"` → `\"`, which doesn't protect against `$()` or backticks inside double quotes.

**Fix:** Use `shell-escape` or `shlex` crate. Wrap `working_dir` in single quotes with embedded `'` escaped as `'\''`. Apply same treatment to prompt.

---

### C3. Octal decoder accepts digits 8 and 9

**File:** `src/tmux/control.rs:40`

`is_ascii_digit()` matches `0-9`, not `0-7`. Sequences like `\890` silently produce corrupt bytes after `u8` truncation.

**Fix:** Change each digit check to `(b'0'..=b'7').contains(&d)`.

---

### C4. `resize(0, _)` panics on `u16` underflow

**File:** `src/session/terminal.rs:119-120`

`rows - 1` wraps to 65535 when `rows = 0`. Panics in debug, silent corruption in release.

**Fix:** Early return at top of `resize` if `cols == 0 || rows == 0`.

---

## Important (8)

### I1. Debounce logic is unreachable (dead code)

**File:** `src/events/queue.rs:108-117`

Fairness cooldown (5s) is checked before debounce (2s). Since cooldown is stricter, it always fires first. Debounce only works in tests where `cooldown = Duration::ZERO`.

**Fix:** Reorder checks — debounce first, then cooldown. Or merge them.

---

### I2. `NeedsAttention` → `Running` transition missing

**File:** `src/app.rs:93-103`

`PreToolUse`/`PostToolUse`/`Notification` only transition from `Starting` or `Running`. A completed session that resumes work stays stuck as "done" in the sidebar forever.

**Fix:** Broaden the guard to transition any non-terminal status (`Starting | Running | NeedsAttention`) to `Running`.

---

### I3. `kill()` uses nickname instead of pane ID

**File:** `src/session/manager.rs:168`

Nicknames aren't unique. `tmux_pane_id` is already stored in `Session` but unused for kills.

**Fix:** Use `tmux kill-pane -t {pane_id}` instead of `kill-window` by nickname.

---

### I4. Terminal resize events dropped

**File:** `src/main.rs:180`

`Event::Resize` from crossterm is silently ignored. Dimensions frozen at startup.

**Fix:** Match `Event::Resize(w, h)`, update `session_manager` dimensions, re-query `terminal.size()`.

---

### I5. `last_entry_time` never pruned

**File:** `src/events/queue.rs:41,148`

Grows by up to 3 entries per historical session, never cleaned up on kill/dismiss.

**Fix:** Add `remove_session(session_id)` to `AttentionQueue` that clears both the queue entry and all `last_entry_time` keys. Call from session kill path.

---

### I6. Stranded `.draining` file causes silent event loss

**File:** `src/events/hook_listener.rs:49-54`

If `read_to_string` fails after rename, `.draining` file is never removed. Next restart can't find `.buffer`, events are permanently lost.

**Fix:** Clean up `.draining` in the error path. On startup, also check for stranded `.draining` files before attempting `.buffer` rename.

---

### I7. `SessionEnd` clears `ToolError` queue entries

**File:** `src/events/queue.rs:88-96`

Any non-queueable event clears error entries. `SessionEnd` arriving before user sees the error silently drops it.

**Fix:** Narrow the clearing condition to specific events (`Notification`, `PreToolUse`) or exclude `SessionEnd`/`SessionStart`.

---

### I8. `Vec::remove(0)` in scroll hot path — O(n)

**File:** `src/session/terminal.rs:131`

Both `scrollback` and `grid` use `Vec`. `remove(0)` shifts all elements on every scroll. O(n) per line of output.

**Fix:** Use `VecDeque` for both. `pop_front()` + `push_back()` is O(1).

---

## Lower Priority

### T1. TOCTOU race in runtime directory creation

**File:** `src/main.rs:31-67`

Between `dir.exists()` returning false and `create_dir_all`, another process could create the directory with permissive mode. The ownership/mode check only runs in the `else` branch.

**Fix:** Always verify ownership and set permissions after `create_dir_all`, regardless of which branch created it. Or use `nix::unistd::mkdir` which accepts mode atomically.

---

## Suggested Fix Order

1. **C1** (hook wiring) — without this, nothing else matters
2. **C2** (shell injection) — security
3. **I1** (debounce reorder) — correctness of queue semantics
4. **I2** (state transition) — sessions getting stuck
5. **C3, C4** (octal, resize) — quick defensive fixes
6. Everything else
