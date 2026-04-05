# herald: Problem Statement

## The Problem

When working with Claude Code on multiple tasks simultaneously, there's no good way to orchestrate concurrent sessions. Today you either:

1. **Work serially** — finish one task before starting another, wasting time waiting on long-running operations
2. **Manual tmux juggling** — open multiple tmux panes with Claude Code sessions and constantly switch between them, losing track of which ones need attention
3. **Miss critical moments** — a session might be waiting for a permission prompt or stuck on an error while you're focused on another pane

There's no unified view of what's happening across sessions, no way to know which session needs you *right now*, and no prioritization when multiple sessions need attention simultaneously.

## The Core Insight

Most Claude Code sessions spend the majority of their time *not* needing human input. They're researching, writing code, running tests. The moments that need human attention are relatively rare but time-critical: permission prompts, error recovery, and reviewing completed work.

This is an **event loop problem**. You should be able to work on one thing at a time while a system monitors all your sessions and surfaces the right one when it needs you.

## What We Want

A terminal UI that acts as a session orchestrator:

- **Launch and manage** multiple Claude Code sessions from one place
- **Monitor status** of all sessions in real-time via a sidebar (running, waiting, errored, done)
- **Surface attention-needed sessions** to a main interaction area using a priority queue (errors > permission prompts > completions)
- **Interact directly** with the active session via an embedded terminal — no context switching to a different pane
- **Stay informed** without staying vigilant — the tool watches so you don't have to

## Success Criteria

- Can run 3-5 concurrent Claude Code sessions without losing track of any
- Permission prompts, errors, and completions are surfaced within seconds
- Interacting with a surfaced session feels like native terminal use (no input lag, full escape sequence support)
- Sessions survive TUI crashes (they persist independently)

## Constraints

- Must work on macOS (primary) and Linux
- Claude Code hooks are the detection mechanism (officially supported, structured events)
- tmux is an acceptable dependency (widely available, handles terminal emulation)
- Built in Rust (ratatui + crossterm)
