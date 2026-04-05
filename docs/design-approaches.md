# herald: Design Approaches

## What We're Building

A TUI wrapper around Claude Code that manages multiple concurrent sessions. Think of it as a session orchestrator with:

- **Main area**: Embedded terminal showing the active Claude Code session you're interacting with
- **Sidebar**: List of all active sessions with nicknames and live status (in progress, looping, researching, etc.)
- **Priority event queue**: When sessions need attention (errors > permission prompts > completions), they surface to the main area automatically
- **Session launching**: Spawn new Claude Code sessions from within the TUI

**Tech stack**: Rust with ratatui + crossterm

**State detection**: Claude Code hooks (PreToolUse, PostToolUse, Notification, Stop) emit structured events via Unix domain socket to the dashboard.

---

## Approach A: Direct tmux Orchestrator (Recommended)

The TUI app directly manages tmux sessions via the `tmux` CLI. Each Claude Code session runs in a tmux pane. The main area captures/displays the active pane's content. Claude Code hooks write events to a Unix domain socket that the TUI listens on.

### How it works

1. User launches `herald` which starts the ratatui TUI
2. TUI creates a tmux session behind the scenes for each Claude Code instance
3. Main area renders the content of the currently-focused tmux pane (via `tmux capture-pane`)
4. Keyboard input in the main area is forwarded to the active tmux pane (via `tmux send-keys`)
5. Claude Code hooks fire on events, writing JSON to a Unix socket
6. TUI listens on the socket, updates sidebar status, and manages the priority queue

### Architecture

```
┌─────────────────────────────────────────────────────┐
│  herald TUI (ratatui)                           │
│                                                     │
│  ┌───────────────────────────┐  ┌────────────────┐  │
│  │                           │  │  Sessions       │  │
│  │   Main Area               │  │                │  │
│  │   (captured tmux pane)    │  │  > fix-tests   │  │
│  │                           │  │    [error]     │  │
│  │   $ claude ...            │  │                │  │
│  │   > Allow edit to foo.rs? │  │    refactor    │  │
│  │   [y/n]                   │  │    [running]   │  │
│  │                           │  │                │  │
│  │                           │  │    add-auth    │  │
│  │                           │  │    [done]      │  │
│  └───────────────────────────┘  └────────────────┘  │
│  [New Session]  [Kill]  [Rename]                    │
└─────────────────────────────────────────────────────┘
         │                          ▲
         │ tmux send-keys           │ tmux capture-pane
         ▼                          │
┌─────────────────────────────────────────────────────┐
│  tmux server                                        │
│                                                     │
│  session:0  ──  claude -p "fix tests"               │
│  session:1  ──  claude -p "refactor auth module"    │
│  session:2  ──  claude -p "add auth middleware"     │
└─────────────────────────────────────────────────────┘
         │
         │  hooks emit JSON events
         ▼
┌──────────────────────┐
│  Unix Domain Socket  │
│  /tmp/herald.sock│
└──────────────────────┘
```

### Pros

- Tmux handles all the hard terminal emulation — we don't reimplement it
- Embedded terminal is natural (just capture/display the pane content)
- Hooks give reliable, structured event detection
- Users can fall back to raw tmux if needed (sessions persist)
- Sessions survive if the TUI crashes

### Cons

- Requires tmux as a dependency
- `tmux capture-pane` polling introduces slight latency for main area rendering
- Tight coupling to tmux CLI interface

### Key risks

- **Capture-pane refresh rate**: Polling `tmux capture-pane` at ~30-60fps may have performance implications. May need to tune or use `tmux pipe-pane` for streaming.
- **Input forwarding fidelity**: `tmux send-keys` needs to handle special keys (ctrl sequences, arrows) correctly.

---

## Approach B: PTY Multiplexer (No tmux Dependency)

The TUI app itself spawns Claude Code processes with pseudo-terminals (PTY). It acts as its own terminal multiplexer — no tmux needed. Each session is a child process with a PTY pair. The TUI reads PTY output and renders it directly. Hooks still communicate via Unix socket.

### How it works

1. User launches `herald`
2. For each session, the app forks and creates a PTY pair
3. Claude Code runs in the child process attached to the slave PTY
4. TUI reads from the master PTY and renders terminal output using a VT100 parser
5. Keyboard input is written to the master PTY
6. Hooks work the same as Approach A

### Architecture

```
┌─────────────────────────────────────────────────────┐
│  herald TUI (ratatui)                           │
│                                                     │
│  ┌──────────────────┐  ┌──────────────────┐         │
│  │ VT100 Parser     │  │ VT100 Parser     │  ...    │
│  │ (session 0)      │  │ (session 1)      │         │
│  └────────┬─────────┘  └────────┬─────────┘         │
│           │ read                │ read               │
│  ┌────────▼─────────┐  ┌───────▼──────────┐         │
│  │ PTY master fd    │  │ PTY master fd    │         │
│  └────────┬─────────┘  └───────┬──────────┘         │
└───────────┼─────────────────────┼───────────────────┘
            │ pty pair            │ pty pair
  ┌─────────▼─────────┐  ┌───────▼──────────┐
  │ claude -p "..."   │  │ claude -p "..."  │
  │ (child process)   │  │ (child process)  │
  └───────────────────┘  └──────────────────┘
```

### Pros

- Zero external dependencies (no tmux required)
- Full control over rendering pipeline
- No polling — PTY reads are event-driven
- Potentially smoother rendering

### Cons

- Must implement or integrate a VT100/xterm terminal emulator (parsing escape sequences, colors, cursor movement, alternate screen buffer, etc.)
- Significantly more complex — terminal emulation is notoriously tricky
- Sessions die if the TUI crashes (no fallback)
- Rust PTY crates exist but VT100 parsing at full fidelity is a large surface area

### Key risks

- **Terminal emulation correctness**: Claude Code uses rich terminal output (colors, spinners, clearing lines). A partial VT100 implementation will produce rendering artifacts.
- **Maintenance burden**: Terminal emulation bugs will be an ongoing tax.

---

## Approach C: Thin tmux Wrapper + Separate Dashboard

Two separate tools that communicate:

1. **`herald-launch`** — A shell script/small binary that creates tmux sessions for Claude Code instances with the right hooks configured
2. **`herald-dash`** — A ratatui TUI that reads events from the Unix socket, shows the sidebar, and lets you switch between tmux panes

### How it works

1. User runs `herald-launch "fix tests"` to spawn a session
2. The launcher creates a tmux session, configures hooks, and registers with the dashboard
3. User runs `herald-dash` to open the dashboard
4. Dashboard shows all registered sessions and their status
5. Selecting a session runs `tmux attach` or similar to switch focus

### Architecture

```
Terminal 1:                    Terminal 2 (or tmux pane):
┌────────────────────┐         ┌──────────────────────┐
│ herald-dash    │         │ herald-launch    │
│                    │         │ "fix tests"          │
│ Sessions:          │         └──────────┬───────────┘
│  > fix-tests [err] │                    │
│    refactor  [run] │                    │ creates tmux session
│    add-auth  [done]│                    ▼
│                    │         ┌──────────────────────┐
│ [Enter] to attach  │         │ tmux: claude -p ...  │
└────────────────────┘         └──────────────────────┘
         ▲
         │ reads events
         │
┌────────┴───────────┐
│ Unix Domain Socket │
└────────────────────┘
```

### Pros

- Each piece is simpler and independently testable
- Launcher can be used without the dashboard (just tmux + hooks)
- Dashboard can be restarted without affecting sessions
- More unix-philosophy: small composable tools

### Cons

- No embedded terminal in the dashboard — you switch away to interact with sessions
- Two things to coordinate (launcher registration, socket protocol)
- Less cohesive UX — jumping between dashboard and tmux panes
- The "main area" concept from the original vision doesn't really exist here

### Key risks

- **UX fragmentation**: Switching between the dashboard and tmux panes breaks flow. This is fundamentally at odds with the "event loop on one screen" vision.

---

## Comparison Matrix

| Dimension                  | A: tmux Orchestrator | B: PTY Multiplexer | C: Wrapper + Dashboard |
|----------------------------|----------------------|---------------------|------------------------|
| External dependencies      | tmux required        | None                | tmux required          |
| Implementation complexity  | Medium               | High                | Low                    |
| Terminal rendering fidelity| High (tmux does it)  | Depends on VT100 impl | High (tmux does it) |
| Embedded terminal UX       | Yes                  | Yes                 | No (separate panes)    |
| Session crash resilience   | Sessions survive     | Sessions die        | Sessions survive       |
| Matches original vision    | Fully                | Fully               | Partially              |
| Time to MVP                | ~2-3 weeks           | ~5-8 weeks          | ~1 week                |

## Recommendation

**Approach A (Direct tmux Orchestrator)** best balances the original vision with pragmatism. Tmux handles terminal multiplexing — a solved problem — while we focus on the orchestration UX layer that doesn't exist yet. The hook-based event system gives us reliable state detection without fragile screen scraping.

The main technical risk (capture-pane polling latency) is well-understood and has known mitigations (`pipe-pane`, adaptive refresh rates).
