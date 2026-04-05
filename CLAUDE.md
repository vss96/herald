# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
cargo build --release          # Build optimized binary at ./target/release/herald
cargo run                      # Debug build + run
cargo test                     # All 177 tests (~0.06s)
cargo test queue::             # Run tests for a specific module
cargo test test_debounce       # Run a single test by name
cargo test -- --test-threads=1 # Serial execution (useful for debugging)
cargo clippy                   # Lint checks
cargo insta review             # Review snapshot test changes
```

**Prerequisites:** tmux 3.0+, Claude Code CLI (`claude`) in PATH, Rust 1.92+.

## Architecture

Herald is a TUI session orchestrator that manages multiple concurrent Claude Code sessions from a single terminal. It auto-surfaces sessions needing human attention (errors > permission prompts > completions).

### Core Data Flow

1. **Session launch** — `SessionManager::launch()` creates a tmux window, writes hook config to `~/.claude/settings.local.json`, sends `claude` command, spawns a `HookListener`
2. **Event reception** — Claude Code fires hook events → hook script (Python/Bash) forwards via Unix domain socket → `HookListener` deserializes and sends to `AttentionQueue`
3. **Priority queue** — `AttentionQueue` maintains at most one entry per session, with priority replacement, error debounce (2s), and fairness cooldown (5s per tier)
4. **Rendering** — Event loop (`main.rs`) uses `tokio::select!` over keyboard events, hook events, and render ticks. `App::render()` draws sidebar + main area via ratatui

### Key Modules

| Module | Responsibility |
|--------|---------------|
| `src/app.rs` | Central state, key routing, focus management (Sidebar/Main/Dialog), rendering orchestration |
| `src/session/manager.rs` | Session lifecycle (launch/kill), hook config generation, tmux session discovery |
| `src/session/terminal.rs` | VTE terminal emulator — parses tmux control mode output into a styled cell grid |
| `src/events/queue.rs` | Priority-based attention queue with debounce, fairness, and stale entry expiry |
| `src/events/hook_listener.rs` | Unix socket listener with buffer-file recovery for crash resilience |
| `src/events/status_mapper.rs` | Pure function mapping hook events → SessionStatus transitions |
| `src/config.rs` | TOML keybinding config loader (`herald.toml`) |
| `src/input/batcher.rs` | Keystroke batching — literal keys batched and flushed after 8ms to reduce tmux spawns |
| `src/input/tmux_keys.rs` | Key-to-tmux escape sequence mapping |
| `src/tui/sidebar.rs` | Session list widget with status-colored indicators |
| `src/tui/dialogs.rs` | New session dialog with Tab field cycling and path completion |
| `src/tmux/control.rs` | Parses tmux control mode `%output` events, handles escape decoding |
| `scripts/herald-hook.py` | Primary hook script (Python); `herald-hook.sh` is the bash fallback |

### SessionStatus State Machine

Defined in `src/session/model.rs`, transitions driven by `src/events/status_mapper.rs`:

```
Starting → Running (on any tool activity or notification)
Running → NeedsAttention(PermissionPrompt) (on PermissionRequest event)
Running → NeedsAttention(ToolError) (on PostToolUseFailure)
Running → NeedsAttention(Completed) (on Stop event)
NeedsAttention → Running (on new tool activity — user responded)
Any → Error (on unrecoverable failure)
```

### Focus & Input Routing

`App` uses a `Focus` enum (`Sidebar | MainArea | Dialog`) to route all keyboard input:

- **Sidebar** — session navigation (j/k/Enter), new session (n), kill (x), dismiss (d), quit (q)
- **MainArea** — all keys forwarded to tmux pane except `Ctrl+G` (return to sidebar)
- **Dialog** — text field editing with Tab for field cycling and path completion

### Runtime Directory

Per-session sockets and buffer files live in a platform-specific runtime dir:
- **Linux:** `$XDG_RUNTIME_DIR/herald/`
- **macOS:** `$TMPDIR/herald-<uid>/` or `/tmp/herald-<uid>/`

Directory and files are created with restrictive permissions (0700/0600) and ownership is verified on startup.

### Patterns

- **Async:** Tokio multi-threaded runtime. tmux commands use `spawn_blocking`. Event loop uses `tokio::select!`.
- **Error handling:** `anyhow::Result<T>` with `.context()` chaining throughout.
- **UI:** Stateless immediate-mode rendering with ratatui. No retained widget state.
- **State machine:** `SessionStatus` enum drives session lifecycle and auto-switch behavior (3s idle threshold).
- **Tests:** Co-located `mod tests {}` blocks within each source file. Snapshot tests use `insta` crate (`src/snapshots/`).

### Logging

Structured logs (via `tracing`) write to `~/.local/share/herald/logs/`. Useful for debugging hook event flow and tmux interactions.

### Design Docs

Architectural context and decision records live in `docs/` (problem statement, full spec, design approaches).

## Git Commits

- Never add `Co-Authored-By` lines to commit messages
