use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use tokio::sync::mpsc;

use crate::events::queue::AttentionQueue;
use crate::events::types::HookEvent;
use crate::session::manager::SessionManager;
use crate::session::model::SessionStatus;
use crate::tui::dialogs::NewSessionDialog;
use crate::tui::layout;
use crate::tui::main_area::MainArea;
use crate::tui::sidebar::Sidebar;

/// Which part of the UI has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Sidebar,
    MainArea,
    Dialog,
}

/// Central application state.
pub struct App {
    pub session_manager: SessionManager,
    pub attention_queue: AttentionQueue,
    pub active_session_id: Option<String>,
    pub focus: Focus,
    pub sidebar_index: usize,
    pub dialog: NewSessionDialog,
    pub last_keypress: Instant,
    pub should_quit: bool,
    pub runtime_dir: PathBuf,
    /// Raw captured pane content for the active session (rendered directly)
    pub captured_content: Option<String>,
    /// Session ID pending kill (processed async in the event loop)
    pub pending_kill: Option<String>,
    /// Channel for sending events into the main loop (used to spawn hook listeners)
    pub event_tx: Option<mpsc::UnboundedSender<crate::AppEvent>>,
    /// Receiver end — moved into the run_loop
    pub event_rx: mpsc::UnboundedReceiver<crate::AppEvent>,
    idle_threshold: Duration,
}

impl App {
    pub fn new(runtime_dir: PathBuf, terminal_cols: u16, terminal_rows: u16) -> Self {
        // Create a dummy channel — main() will replace event_tx
        let (_tx, rx) = mpsc::unbounded_channel();
        Self {
            session_manager: SessionManager::new(runtime_dir.clone(), terminal_cols, terminal_rows),
            attention_queue: AttentionQueue::new(),
            active_session_id: None,
            focus: Focus::Sidebar,
            sidebar_index: 0,
            dialog: NewSessionDialog::default(),
            last_keypress: Instant::now(),
            should_quit: false,
            runtime_dir,
            captured_content: None,
            pending_kill: None,
            event_tx: None,
            event_rx: rx,
            idle_threshold: Duration::from_secs(3),
        }
    }

    /// Process a hook event from a Claude Code session.
    pub fn handle_hook_event(&mut self, event: HookEvent) {
        let session_id = event.session_id.clone();

        // Ignore events from sessions we don't manage (stale buffer events)
        if self.session_manager.get(&session_id).is_none() {
            return;
        }

        tracing::info!(
            session_id = %session_id,
            event = ?event.hook_event_name,
            tool = ?event.tool_name,
            "hook event received"
        );
        let changed = self.attention_queue.process_event(&event);

        // Update session status based on event
        if let Some(session) = self.session_manager.get_mut(&session_id) {
            match event.hook_event_name {
                crate::events::types::HookEventName::PermissionRequest => {
                    session.status = SessionStatus::NeedsAttention {
                        reason: crate::session::model::AttentionReason::PermissionPrompt {
                            tool_name: event.tool_name.clone().unwrap_or_default(),
                        },
                        since: Instant::now(),
                    };
                }
                crate::events::types::HookEventName::PostToolUseFailure => {
                    session.status = SessionStatus::NeedsAttention {
                        reason: crate::session::model::AttentionReason::ToolError {
                            tool_name: event.tool_name.clone().unwrap_or_default(),
                            error: String::new(),
                        },
                        since: Instant::now(),
                    };
                }
                crate::events::types::HookEventName::Stop => {
                    session.status = SessionStatus::NeedsAttention {
                        reason: crate::session::model::AttentionReason::Completed,
                        since: Instant::now(),
                    };
                }
                crate::events::types::HookEventName::PostToolUse
                | crate::events::types::HookEventName::PreToolUse
                | crate::events::types::HookEventName::Notification => {
                    // Transition any non-terminal status to Running
                    match session.status {
                        SessionStatus::Starting
                        | SessionStatus::Running { .. }
                        | SessionStatus::NeedsAttention { .. } => {
                            session.status = SessionStatus::Running {
                                last_activity: Instant::now(),
                            };
                        }
                        _ => {} // Don't resurrect Stopped/Error sessions
                    }
                }
                _ => {}
            }
        }

        // Auto-switch to highest priority session if user is idle
        if changed {
            self.try_auto_switch();
        }
    }

    /// Handle a keyboard event.
    ///
    /// When main area is focused, ALL keys go to the tmux pane (except Esc).
    /// When sidebar is focused, keys control herald (n/x/q/j/k/Enter/etc).
    /// Ctrl-C only quits herald from the sidebar — in the main area it goes to Claude.
    pub async fn handle_key(&mut self, key: KeyEvent) {
        self.last_keypress = Instant::now();

        match self.focus {
            Focus::MainArea => self.handle_main_area_key(key),
            Focus::Dialog => self.handle_dialog_key(key).await,
            Focus::Sidebar => {
                // Ctrl-C quits herald only from sidebar
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    self.should_quit = true;
                    return;
                }
                self.handle_sidebar_key(key);
            }
        }
    }

    fn handle_sidebar_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('n') => {
                self.dialog.visible = true;
                self.dialog.working_dir.set(
                    std::env::current_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                );
                self.focus = Focus::Dialog;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let count = self.session_manager.session_count();
                if count > 0 {
                    self.sidebar_index = (self.sidebar_index + 1) % count;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let count = self.session_manager.session_count();
                if count > 0 {
                    self.sidebar_index = (self.sidebar_index + count - 1) % count;
                }
            }
            KeyCode::Enter => {
                // Focus the selected session in main area
                if let Some(session) = self.session_ids().get(self.sidebar_index) {
                    self.active_session_id = Some(session.clone());
                    self.captured_content = None; // Force refresh on next tick
                    self.focus = Focus::MainArea;
                }
            }
            KeyCode::Tab => {
                if self.active_session_id.is_some() {
                    self.focus = Focus::MainArea;
                }
            }
            KeyCode::Char('d') => {
                // Dismiss completion/error for selected session
                if let Some(id) = self.session_ids().get(self.sidebar_index).cloned() {
                    self.attention_queue.dismiss_completion(&id);
                    self.attention_queue.dismiss_error(&id);
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                // Kill the selected session
                if let Some(id) = self.session_ids().get(self.sidebar_index).cloned() {
                    self.pending_kill = Some(id);
                }
            }
            _ => {}
        }
    }

    fn handle_main_area_key(&mut self, key: KeyEvent) {
        // Esc always returns to sidebar
        if key.code == KeyCode::Esc {
            self.focus = Focus::Sidebar;
            return;
        }

        // Forward all other keys to the active tmux pane
        if let Some(ref session_id) = self.active_session_id {
            if let Some(session) = self.session_manager.get(session_id) {
                let pane_id = session.tmux_pane_id.clone();
                // Spawn the send as a background task (non-blocking)
                match key.code {
                    KeyCode::Enter => {
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_special_key(&pane_id, "Enter").await;
                        });
                    }
                    KeyCode::Backspace => {
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_special_key(&pane_id, "BSpace").await;
                        });
                    }
                    KeyCode::Up => {
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_special_key(&pane_id, "Up").await;
                        });
                    }
                    KeyCode::Down => {
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_special_key(&pane_id, "Down").await;
                        });
                    }
                    KeyCode::Left => {
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_special_key(&pane_id, "Left").await;
                        });
                    }
                    KeyCode::Right => {
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_special_key(&pane_id, "Right").await;
                        });
                    }
                    KeyCode::Tab => {
                        // Shift+Tab → send BTab (BackTab) to tmux
                        let tmux_key = if key.modifiers.contains(KeyModifiers::SHIFT) {
                            "BTab"
                        } else {
                            "Tab"
                        };
                        let tmux_key = tmux_key.to_string();
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_special_key(&pane_id, &tmux_key).await;
                        });
                    }
                    KeyCode::BackTab => {
                        // BackTab is Shift+Tab on some terminals
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_special_key(&pane_id, "BTab").await;
                        });
                    }
                    KeyCode::Char(c) => {
                        let ch = if key.modifiers.contains(KeyModifiers::SHIFT | KeyModifiers::CONTROL) {
                            format!("C-S-{}", c)
                        } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                            format!("C-{}", c)
                        } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                            // Shift+char — just send the char (uppercase handled by terminal)
                            c.to_string()
                        } else {
                            c.to_string()
                        };
                        tokio::spawn(async move {
                            let _ = crate::tmux::commands::send_keys_literal(&pane_id, &ch).await;
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    async fn handle_dialog_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.dialog.reset();
                self.focus = Focus::Sidebar;
            }
            KeyCode::Tab => {
                // Tab on directory field: path completion
                if self.dialog.active_field == crate::tui::dialogs::DialogField::WorkingDir {
                    self.complete_directory_path();
                } else {
                    self.dialog.next_field();
                }
            }
            KeyCode::Enter => {
                if self.dialog.active_field == crate::tui::dialogs::DialogField::Prompt
                    && self.dialog.is_valid()
                {
                    // Submit on last field when valid
                    let nickname = self.dialog.nickname.text.clone();
                    let prompt = self.dialog.prompt.text.clone();
                    let working_dir = PathBuf::from(&self.dialog.working_dir.text);
                    tracing::info!(
                        nickname = %nickname,
                        prompt = %prompt,
                        dir = %working_dir.display(),
                        "launching new session"
                    );
                    self.dialog.reset();
                    self.focus = Focus::Sidebar;

                    match self.session_manager.launch(&nickname, &prompt, &working_dir).await {
                        Ok(id) => {
                            tracing::info!(session_id = %id, "session launched");
                            // Spawn a hook listener for this new session
                            if let Some(ref tx) = self.event_tx {
                                crate::spawn_hook_listener(
                                    &self.runtime_dir,
                                    &id,
                                    tx.clone(),
                                );
                            }
                            self.active_session_id = Some(id);
                            self.focus = Focus::MainArea;
                        }
                        Err(e) => {
                            tracing::error!("failed to launch session: {}", e);
                        }
                    }
                } else {
                    // Enter advances to the next field
                    self.dialog.next_field();
                }
            }
            KeyCode::Backspace => {
                self.dialog.active_input().backspace();
            }
            KeyCode::Delete => {
                self.dialog.active_input().delete();
            }
            KeyCode::Left => {
                self.dialog.active_input().move_left();
            }
            KeyCode::Right => {
                self.dialog.active_input().move_right();
            }
            KeyCode::Home => {
                self.dialog.active_input().home();
            }
            KeyCode::End => {
                self.dialog.active_input().end();
            }
            KeyCode::Char(c) => {
                self.dialog.active_input().insert(c);
            }
            _ => {}
        }
    }

    /// Tab-complete directory paths (like a terminal).
    fn complete_directory_path(&mut self) {
        let input_text = self.dialog.working_dir.text.clone();
        let path = PathBuf::from(&input_text);

        let (search_dir, prefix) = if input_text.ends_with('/') || input_text.ends_with(std::path::MAIN_SEPARATOR) {
            (path.clone(), String::new())
        } else {
            let parent = path.parent().unwrap_or_else(|| std::path::Path::new("/"));
            let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            (parent.to_path_buf(), file_name)
        };

        let Ok(entries) = std::fs::read_dir(&search_dir) else {
            return;
        };

        let mut matches: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map_or(false, |ft| ft.is_dir()))
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') && !prefix.starts_with('.') {
                    return false;
                }
                name.starts_with(&prefix)
            })
            .map(|e| e.path())
            .collect();

        matches.sort();

        if matches.len() == 1 {
            let completed = format!("{}/", matches[0].display());
            self.dialog.working_dir.set(completed);
        } else if matches.len() > 1 {
            let names: Vec<String> = matches
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            if let Some(common) = longest_common_prefix(&names) {
                self.dialog.working_dir.set(common);
            }
        }
    }

    /// Process any pending session kill.
    pub async fn process_pending_kill(&mut self) {
        let Some(id) = self.pending_kill.take() else {
            return;
        };
        tracing::info!(session_id = %id, "killing session");
        if let Err(e) = self.session_manager.kill(&id).await {
            tracing::error!(session_id = %id, "failed to kill session: {}", e);
        }
        self.attention_queue.dismiss_error(&id);
        self.attention_queue.dismiss_completion(&id);

        // Fix active session and sidebar index
        if self.active_session_id.as_deref() == Some(&id) {
            self.active_session_id = self.session_ids().first().cloned();
            self.captured_content = None;
        }
        let count = self.session_manager.session_count();
        if count > 0 {
            self.sidebar_index = self.sidebar_index.min(count - 1);
        } else {
            self.sidebar_index = 0;
        }
    }

    /// Drain buffer files for all sessions — fallback for when socket delivery fails.
    pub async fn drain_all_buffers(&mut self) {
        let session_ids: Vec<String> = self
            .session_manager
            .sessions()
            .map(|s| s.id.clone())
            .collect();

        for sid in session_ids {
            let listener = crate::events::hook_listener::HookListener::new(
                self.session_manager.runtime_dir(),
                &sid,
            );
            match listener.drain_buffer().await {
                Ok(events) if !events.is_empty() => {
                    tracing::info!(session_id = %sid, count = events.len(), "drained buffer events");
                    for event in events {
                        self.handle_hook_event(event);
                    }
                }
                _ => {}
            }
        }
        // Check if any drained events should trigger auto-switch
        self.try_auto_switch();
    }

    /// Refresh the active session's pane content from tmux capture-pane.
    pub async fn refresh_active_terminal(&mut self) {
        if self.focus == Focus::Dialog {
            return;
        }

        let Some(ref session_id) = self.active_session_id else {
            self.captured_content = None;
            return;
        };
        let pane_id = match self.session_manager.get(session_id) {
            Some(s) if !s.tmux_pane_id.is_empty() => s.tmux_pane_id.clone(),
            _ => {
                self.captured_content = None;
                return;
            }
        };

        match crate::tmux::commands::capture_pane(&pane_id).await {
            Ok(content) => {
                self.captured_content = if content.is_empty() { None } else { Some(content) };
            }
            Err(e) => {
                tracing::warn!(pane_id = %pane_id, "capture-pane failed: {}", e);
            }
        }
    }

    /// Auto-switch to a session that needs attention.
    /// Only switches if: user is idle AND not manually viewing a session.
    /// This prevents yanking the user away from a session they deliberately selected.
    fn try_auto_switch(&mut self) {
        if self.focus == Focus::Dialog {
            return;
        }
        // Don't auto-switch if user is in the main area (they chose to be there)
        if self.focus == Focus::MainArea {
            return;
        }
        // Only auto-switch if user is idle
        if !self.is_idle() {
            return;
        }
        if let Some(entry) = self.attention_queue.peek() {
            self.active_session_id = Some(entry.session_id.clone());
            if let Some(idx) = self.session_ids().iter().position(|id| id == &entry.session_id) {
                self.sidebar_index = idx;
            }
            self.focus = Focus::MainArea;
        }
    }

    fn is_idle(&self) -> bool {
        Instant::now().duration_since(self.last_keypress) > self.idle_threshold
    }

    /// Get sorted session IDs (stable ordering for sidebar).
    fn session_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .session_manager
            .sessions()
            .map(|s| s.id.clone())
            .collect();
        ids.sort();
        ids
    }

    fn render_dialog(&self, area: Rect, buf: &mut Buffer) {
        // Center the dialog
        let dialog_width = 60u16.min(area.width.saturating_sub(4));
        let dialog_height = 11u16;
        let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
        let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

        // Clear the area behind the dialog
        for row in dialog_area.y..dialog_area.y + dialog_area.height {
            for col in dialog_area.x..dialog_area.x + dialog_area.width {
                if let Some(cell) = buf.cell_mut((col, row)) {
                    cell.set_char(' ');
                    cell.set_style(Style::default());
                }
            }
        }

        // Draw border
        let block = ratatui::widgets::Block::default()
            .title(" New Session ")
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(dialog_area);
        Widget::render(block, dialog_area, buf);

        let fields: Vec<(&str, &crate::tui::dialogs::TextInput, bool)> = vec![
            ("Nickname", &self.dialog.nickname, self.dialog.active_field == crate::tui::dialogs::DialogField::Nickname),
            ("Directory", &self.dialog.working_dir, self.dialog.active_field == crate::tui::dialogs::DialogField::WorkingDir),
            ("Prompt", &self.dialog.prompt, self.dialog.active_field == crate::tui::dialogs::DialogField::Prompt),
        ];

        for (i, (label, input, active)) in fields.iter().enumerate() {
            let y = inner.y + (i as u16) * 2;
            if y >= inner.y + inner.height {
                break;
            }

            let label_style = if *active {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            buf.set_string(inner.x, y, format!(" {}:", label), label_style);

            let input_x = inner.x + 1;
            let input_y = y + 1;
            if input_y < inner.y + inner.height {
                let val_style = if *active {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::Gray)
                };
                let cursor_style = Style::default().fg(Color::Black).bg(Color::White);

                if *active {
                    let (before, at, after) = input.parts();
                    buf.set_string(input_x, input_y, format!(" {}", before), val_style);
                    let cursor_x = input_x + 1 + before.len() as u16;
                    let cursor_ch = at.unwrap_or(' ');
                    buf.set_string(cursor_x, input_y, cursor_ch.to_string(), cursor_style);
                    if let Some(c) = at {
                        let after_x = cursor_x + c.len_utf8() as u16;
                        buf.set_string(after_x, input_y, after, val_style);
                    }
                } else {
                    buf.set_string(input_x, input_y, format!(" {}", &input.text), val_style);
                }
            }
        }

        // Footer — context-sensitive help
        let footer_y = dialog_area.y + dialog_area.height - 1;
        if footer_y > dialog_area.y {
            let help = if self.dialog.active_field == crate::tui::dialogs::DialogField::WorkingDir {
                " Enter:next  Tab:complete path  Esc:cancel"
            } else if self.dialog.active_field == crate::tui::dialogs::DialogField::Prompt {
                " Enter:launch  Tab:next field  Esc:cancel"
            } else {
                " Enter:next  Tab:next field  Esc:cancel"
            };
            buf.set_string(
                inner.x,
                footer_y - 1,
                help,
                Style::default().fg(Color::DarkGray),
            );
        }
    }

    /// Render the full UI.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let (main_area, sidebar_area) = layout::split_main_sidebar(area);
        let (content_area, status_area) = layout::split_content_status(main_area);

        // Render sidebar
        let session_ids = self.session_ids();
        let sessions: Vec<&crate::session::model::Session> = session_ids
            .iter()
            .filter_map(|id| self.session_manager.get(id))
            .collect();
        let sidebar = Sidebar::new(
            &sessions,
            self.active_session_id.as_deref(),
            self.sidebar_index,
            self.focus == Focus::Sidebar,
        );
        Widget::render(sidebar, sidebar_area, buf);

        // Render main area with raw captured content
        let title = if let Some(ref id) = self.active_session_id {
            self.session_manager
                .get(id)
                .map(|s| s.nickname.clone())
                .unwrap_or_else(|| "herald".to_string())
        } else {
            "herald".to_string()
        };
        let main = MainArea::new(self.captured_content.clone(), title);
        Widget::render(main, content_area, buf);

        // Render dialog overlay if visible
        if self.dialog.visible {
            self.render_dialog(area, buf);
        }

        // Render status bar
        let focus_label = match self.focus {
            Focus::Sidebar => "SIDEBAR",
            Focus::MainArea => "TERMINAL",
            Focus::Dialog => "NEW SESSION",
        };
        // Only count attention entries for sessions that actually exist
        let queue_count = self
            .attention_queue
            .entries_sorted()
            .iter()
            .filter(|e| self.session_manager.get(&e.session_id).is_some())
            .count();
        let bg = Color::Rgb(30, 30, 46); // dark purple-gray
        let status_line = Line::from(vec![
            Span::styled(" herald ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} ", focus_label), Style::default().fg(Color::Cyan).bg(bg)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray).bg(bg)),
            Span::styled(
                format!("{} sessions", self.session_manager.session_count()),
                Style::default().fg(Color::White).bg(bg),
            ),
            Span::styled(" | ", Style::default().fg(Color::DarkGray).bg(bg)),
            if queue_count > 0 {
                Span::styled(
                    format!("{} need attention", queue_count),
                    Style::default().fg(Color::Red).bg(bg).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("all clear", Style::default().fg(Color::Green).bg(bg))
            },
            Span::styled(" | ", Style::default().fg(Color::DarkGray).bg(bg)),
            Span::styled("q:quit n:new x:kill Esc:sidebar", Style::default().fg(Color::DarkGray).bg(bg)),
        ]);
        // Fill the rest of the status bar with background
        buf.set_style(status_area, Style::default().bg(bg));
        buf.set_line(status_area.x, status_area.y, &status_line, status_area.width);
    }
}

/// Find the longest common prefix among a list of strings.
fn longest_common_prefix(strings: &[String]) -> Option<String> {
    if strings.is_empty() {
        return None;
    }
    let first = &strings[0];
    let mut prefix_len = first.len();
    for s in &strings[1..] {
        prefix_len = prefix_len.min(s.len());
        for (i, (a, b)) in first.bytes().zip(s.bytes()).enumerate() {
            if a != b {
                prefix_len = prefix_len.min(i);
                break;
            }
        }
    }
    if prefix_len == 0 {
        None
    } else {
        Some(first[..prefix_len].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::types::{HookEvent, HookEventName};

    fn make_app() -> App {
        App::new(PathBuf::from("/tmp/herald-test"), 80, 24)
    }

    fn add_fake_session(app: &mut App, session_id: &str) {
        use crate::session::model::Session;
        let s = Session::new(
            session_id.to_string(),
            "test".to_string(),
            "prompt".to_string(),
            PathBuf::from("/tmp"),
            80, 24,
        );
        app.session_manager.insert_test_session(s);
    }

    fn make_hook(session_id: &str, name: HookEventName) -> HookEvent {
        HookEvent {
            session_id: session_id.to_string(),
            hook_event_name: name,
            tool_name: Some("Edit".to_string()),
            tool_use_id: Some("t1".to_string()),
            tool_input: None,
            cwd: None,
        }
    }

    #[test]
    fn initial_state() {
        let app = make_app();
        assert!(app.active_session_id.is_none());
        assert_eq!(app.focus, Focus::Sidebar);
        assert!(!app.should_quit);
    }

    #[tokio::test]
    async fn quit_on_q() {
        let mut app = make_app();
        app.handle_key(KeyEvent::from(KeyCode::Char('q'))).await;
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn quit_on_ctrl_c() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).await;
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn n_opens_dialog() {
        let mut app = make_app();
        app.handle_key(KeyEvent::from(KeyCode::Char('n'))).await;
        assert_eq!(app.focus, Focus::Dialog);
        assert!(app.dialog.visible);
    }

    #[tokio::test]
    async fn esc_closes_dialog() {
        let mut app = make_app();
        app.focus = Focus::Dialog;
        app.dialog.visible = true;
        app.handle_key(KeyEvent::from(KeyCode::Esc)).await;
        assert_eq!(app.focus, Focus::Sidebar);
        assert!(!app.dialog.visible);
    }

    #[tokio::test]
    async fn esc_returns_to_sidebar_from_main() {
        let mut app = make_app();
        app.focus = Focus::MainArea;
        app.handle_key(KeyEvent::from(KeyCode::Esc)).await;
        assert_eq!(app.focus, Focus::Sidebar);
    }

    #[test]
    fn hook_event_queues_permission() {
        let mut app = make_app();
        add_fake_session(&mut app, "s1");
        let event = make_hook("s1", HookEventName::PermissionRequest);

        app.last_keypress = Instant::now() - Duration::from_secs(10);

        app.handle_hook_event(event);
        assert_eq!(app.attention_queue.len(), 1);
    }

    #[test]
    fn auto_switch_when_idle() {
        let mut app = make_app();
        add_fake_session(&mut app, "s1");
        app.last_keypress = Instant::now() - Duration::from_secs(10);

        let event = make_hook("s1", HookEventName::PermissionRequest);
        app.handle_hook_event(event);

        assert_eq!(app.active_session_id, Some("s1".to_string()));
        assert_eq!(app.focus, Focus::MainArea);
    }

    #[test]
    fn no_auto_switch_when_not_idle() {
        let mut app = make_app();
        add_fake_session(&mut app, "s1");
        app.last_keypress = Instant::now(); // just typed = not idle
        app.focus = Focus::Sidebar;

        let event = make_hook("s1", HookEventName::PermissionRequest);
        app.handle_hook_event(event);

        // Event enters queue but no auto-switch (user is active)
        assert_eq!(app.attention_queue.len(), 1);
        assert!(app.active_session_id.is_none());
        assert_eq!(app.focus, Focus::Sidebar);
    }

    #[test]
    fn no_auto_switch_in_dialog() {
        let mut app = make_app();
        app.focus = Focus::Dialog;
        app.last_keypress = Instant::now() - Duration::from_secs(10);

        let event = make_hook("s1", HookEventName::PermissionRequest);
        app.handle_hook_event(event);

        // Should NOT auto-switch while in dialog
        assert!(app.active_session_id.is_none());
    }
}
