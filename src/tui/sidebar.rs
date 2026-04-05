use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Widget};

use crate::session::model::{AttentionReason, Session, SessionStatus};

// Color palette
const BLUE: Color = Color::Rgb(100, 149, 237); // Cornflower blue
const RED: Color = Color::Rgb(255, 85, 85); // Bright red
const GREEN: Color = Color::Rgb(80, 200, 120); // Emerald green
const YELLOW: Color = Color::Rgb(255, 200, 50); // Warm yellow
const DIM: Color = Color::Rgb(100, 100, 100); // Dim gray
const CYAN: Color = Color::Cyan;

/// Sidebar widget showing all sessions with status.
pub struct Sidebar<'a> {
    sessions: &'a [&'a Session],
    active_session_id: Option<&'a str>,
    selected_index: usize,
    has_focus: bool,
}

impl<'a> Sidebar<'a> {
    pub fn new(
        sessions: &'a [&'a Session],
        active_session_id: Option<&'a str>,
        selected_index: usize,
        has_focus: bool,
    ) -> Self {
        Self {
            sessions,
            active_session_id,
            selected_index,
            has_focus,
        }
    }
}

impl<'a> Widget for Sidebar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = format!(" Sessions ({}) ", self.sessions.len());

        // Count sessions needing attention for border color
        let attention_count = self
            .sessions
            .iter()
            .filter(|s| matches!(s.status, SessionStatus::NeedsAttention { .. }))
            .count();

        let border_color = if attention_count > 0 {
            RED
        } else if self.has_focus {
            CYAN
        } else {
            DIM
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let is_active = self
                    .active_session_id
                    .map_or(false, |id| id == session.id);
                let is_selected = i == self.selected_index;

                let (indicator, session_color) = session_style(&session.status);

                // Background for selected item
                let bg = if is_selected && self.has_focus {
                    Some(Color::Rgb(45, 45, 65))
                } else {
                    None
                };

                let name_style = {
                    let mut s = Style::default().fg(session_color);
                    if is_active {
                        s = s.add_modifier(Modifier::BOLD);
                    }
                    if let Some(bg) = bg {
                        s = s.bg(bg);
                    }
                    s
                };

                let indicator_style = {
                    let mut s = Style::default().fg(session_color);
                    if let Some(bg) = bg {
                        s = s.bg(bg);
                    }
                    s
                };

                let label = session.status_label();
                let label_color = match &session.status {
                    SessionStatus::NeedsAttention { reason, .. } => match reason {
                        AttentionReason::ToolError { .. }
                        | AttentionReason::PermissionPrompt { .. } => RED,
                        AttentionReason::Completed => GREEN,
                    },
                    SessionStatus::Running { .. } => BLUE,
                    SessionStatus::Error { .. } => RED,
                    _ => DIM,
                };
                let label_style = {
                    let mut s = Style::default().fg(label_color);
                    if matches!(
                        session.status,
                        SessionStatus::NeedsAttention {
                            reason: AttentionReason::PermissionPrompt { .. }
                                | AttentionReason::ToolError { .. },
                            ..
                        }
                    ) {
                        s = s.add_modifier(Modifier::BOLD);
                    }
                    if let Some(bg) = bg {
                        s = s.bg(bg);
                    }
                    s
                };

                let active_marker = if is_active { "▸" } else { " " };
                let marker_style = {
                    let mut s = Style::default().fg(CYAN);
                    if let Some(bg) = bg {
                        s = s.bg(bg);
                    }
                    s
                };

                let pad_style = {
                    let mut s = Style::default();
                    if let Some(bg) = bg {
                        s = s.bg(bg);
                    }
                    s
                };

                let line = Line::from(vec![
                    Span::styled(active_marker, marker_style),
                    Span::styled(format!("{} ", indicator), indicator_style),
                    Span::styled(&session.nickname, name_style),
                    Span::styled(" ", pad_style),
                    Span::styled(format!("[{}]", label), label_style),
                ]);

                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(block);
        Widget::render(list, area, buf);
    }
}

fn session_style(status: &SessionStatus) -> (&'static str, Color) {
    match status {
        SessionStatus::Starting => ("○", DIM),
        SessionStatus::Running { .. } => ("◐", BLUE),
        SessionStatus::NeedsAttention { reason, .. } => match reason {
            AttentionReason::ToolError { .. } => ("●", RED),
            AttentionReason::PermissionPrompt { .. } => ("●", RED),
            AttentionReason::Completed => ("✓", GREEN),
        },
        SessionStatus::Stopped { .. } => ("✓", GREEN),
        SessionStatus::Error { .. } => ("✗", RED),
    }
}
