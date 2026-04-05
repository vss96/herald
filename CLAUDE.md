# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
cargo build --release          # Build optimized binary at ./target/release/herald
cargo run                      # Debug build + run
cargo test                     # All 88 tests (~0.06s)
cargo test queue::             # Run tests for a specific module
```

**Prerequisites:** tmux 3.0+, Claude Code CLI (`claude`) in PATH, Rust 1.92+.

## Architecture

Herald is a TUI session orchestrator that manages multiple concurrent Claude Code sessions from a single terminal. It auto-surfaces sessions needing human attention (errors > permission prompts > completions).

### Core Data Flow

1. **Session launch** — `SessionManager::launch()` creates a tmux window, writes hook config to `~/.claude/settings.json`, sends `claude` command, spawns a `HookListener`
2. **Event reception** — Claude Code fires hook events → hook script (Python/Bash) forwards via Unix domain socket → `HookListener` deserializes and sends to `AttentionQueue`
3. **Priority queue** — `AttentionQueue` maintains at most one entry per session, with priority replacement, error debounce (2s), and fairness cooldown (5s per tier)
4. **Rendering** — Event loop (`main.rs`) uses `tokio::select!` over keyboard events, hook events, and render ticks. `App::render()` draws sidebar + main area via ratatui

### Key Modules

| Module | Responsibility |
|--------|---------------|
| `src/app.rs` | Central state, key routing, focus management (Sidebar/Main), rendering orchestration |
| `src/session/manager.rs` | Session lifecycle (launch/kill), hook config generation, tmux session discovery |
| `src/session/terminal.rs` | VTE terminal emulator — parses tmux control mode output into a styled cell grid |
| `src/events/queue.rs` | Priority-based attention queue with debounce, fairness, and stale entry expiry |
| `src/events/hook_listener.rs` | Unix socket listener with buffer-file recovery for crash resilience |
| `src/tmux/control.rs` | Parses tmux control mode `%output` events, handles escape decoding |
| `scripts/herald-hook.py` | Primary hook script (Python); `herald-hook.sh` is the bash fallback |

### Patterns

- **Async:** Tokio multi-threaded runtime. tmux commands and socket I/O use `spawn_blocking`. Event loop uses `tokio::select!`.
- **Error handling:** `anyhow::Result<T>` with `.context()` chaining throughout.
- **UI:** Stateless immediate-mode rendering with ratatui. No retained widget state.
- **State machine:** `SessionStatus` enum drives session lifecycle (Starting → Running → NeedsAttention/Stopped/Error).
- **Tests:** Co-located `mod tests {}` blocks within each source file.

## Git Commits

- Never add `Co-Authored-By` lines to commit messages
