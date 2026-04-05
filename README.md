# Herald

```
        /\  ||  /\
       /  \ || /  \
      / /\ \||/ /\ \
     |  \/  \/  \/  |
      \   .-""-.   /
       \ / (00) \ /
        |  \__/  |
       /|""||||""|\
      / |  ||||  | \
     /  | /    \ |  \
    /  /| |    | |\  \
   /__/ | |    | | \__\
        |_|    |_|
    ═══════════════════
        H E R A L D
```

A TUI session orchestrator for [Claude Code](https://docs.anthropic.com/en/docs/claude-code).

Herald lets you run multiple Claude Code sessions from a single terminal. It monitors all sessions in real-time and automatically surfaces the one that needs your attention — a permission prompt, an error, or a completed task. You work on one thing while Herald watches the rest.

## Features

- **Session management** — Launch, kill, and rename Claude Code sessions from a unified dashboard
- **Live sidebar** — See all sessions at a glance with color-coded status indicators (running, permission needed, error, done)
- **Priority-based attention queue** — Errors surface first, then permission prompts, then completions
- **Auto-switch** — When you're idle, Herald automatically brings the most critical session to the foreground
- **Embedded terminal** — Interact directly with the active session; keystrokes are forwarded to Claude Code
- **Session persistence** — Sessions run in tmux, so they survive Herald restarts or crashes
- **Event buffering** — If Herald goes down, hook events buffer to disk and are drained on restart

## Prerequisites

- **tmux** 3.0+ — `brew install tmux` (macOS) or `apt install tmux` (Linux)
- **Claude Code** CLI — `claude` must be available in your PATH
- **Rust** 1.92+ — only needed to build from source

Optional: Python 3 or jq improve hook script reliability, but a bash fallback handles both.

## Quick Start

```bash
# Build
cargo build --release

# Run
./target/release/herald
```

Once Herald is running:

1. Press **`n`** to open the new session dialog
2. Enter a **nickname** (e.g. "fix-auth"), **working directory**, and **prompt**
3. Press **Enter** — Herald launches Claude Code in a tmux window and starts monitoring
4. When a session needs attention, it appears in the main area. Type your response directly.
5. Press **Esc** to return to the sidebar and check on other sessions.

## Keybindings

### Sidebar (default focus)

| Key | Action |
|-----|--------|
| `n` | New session |
| `j` / `↓` | Select next session |
| `k` / `↑` | Select previous session |
| `Enter` | Focus selected session in main area |
| `Tab` | Switch to main area |
| `d` | Dismiss attention alert for selected session |
| `x` / `Delete` | Kill selected session |
| `q` / `Ctrl+c` | Quit Herald (sessions persist in tmux) |

### Main area

| Key | Action |
|-----|--------|
| `Esc` | Return to sidebar |
| Any other key | Forwarded to the active Claude Code session |

## How It Works

Herald combines three mechanisms:

1. **tmux** handles terminal emulation and process lifecycle. Each Claude Code session runs in its own tmux window. Herald connects via tmux control mode to stream output in real-time (no polling).

2. **Claude Code hooks** provide structured events. Herald registers hook scripts that fire on permission requests, tool errors, completions, and other lifecycle events. Events are delivered over Unix domain sockets.

3. **Attention queue** prioritizes what to show you. Events are classified into four tiers:

   | Priority | Events | Behavior |
   |----------|--------|----------|
   | CRITICAL | Tool errors | Surface immediately |
   | HIGH | Permission prompts | Surface immediately |
   | LOW | Session completions | Surface when idle |
   | INFO | Status updates | Sidebar indicator only |

   The queue includes debouncing (2s), per-session fairness cooldowns (5s), and stale entry aging (30s visual indicator, 5min expiry).

## Building from Source

```bash
git clone <repo-url>
cd herald
cargo build --release
```

The binary is at `./target/release/herald`. To install it to your PATH:

```bash
cargo install --path .
```

## Project Structure

```
herald/
├── src/
│   ├── main.rs              # Entry point, terminal setup, event loop
│   ├── app.rs               # App state, key handling, rendering
│   ├── tui/                 # UI components (layout, sidebar, main area, dialogs)
│   ├── session/             # Session model, lifecycle manager, terminal buffer
│   ├── events/              # Hook event types, attention queue, socket listener
│   └── tmux/                # tmux control mode client and CLI wrappers
├── scripts/
│   ├── herald-hook.py       # Hook script (Python, primary)
│   └── herald-hook.sh       # Hook script (Bash, fallback)
└── docs/
    ├── problem-statement.md  # Why Herald exists
    ├── spec.md               # Full technical specification
    └── design-approaches.md  # Architecture decision record
```

## License

[MIT](LICENSE)
