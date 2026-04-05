use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Widget};

use crate::session::model::{Session, SessionStatus, AttentionReason};

/// Sidebar widget showing all sessions with status.
pub struct Sidebar<'a> {
    sessions: &'a [&'a Session],
    active_session_id: Option<&'a str>,
    /// Index of the currently highlighted session in the sidebar
    selected_index: usize,
    /// Whether the sidebar has keyboard focus
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
        let border_color = if self.has_focus {
            Color::Cyan
        } else {
            Color::DarkGray
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

                let (indicator, indicator_color) = status_indicator(&session.status);

                let name_style = if is_selected && self.has_focus {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if is_active {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let label = session.status_label();
                let label_style = if is_selected && self.has_focus {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    status_label_style(&session.status)
                };

                let indicator_style = if is_selected && self.has_focus {
                    Style::default().fg(indicator_color).bg(Color::Cyan)
                } else {
                    Style::default().fg(indicator_color)
                };

                // Show a marker for the active (viewed) session
                let active_marker = if is_active { ">" } else { " " };
                let marker_style = if is_selected && self.has_focus {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::Cyan)
                };

                let line = Line::from(vec![
                    Span::styled(active_marker, marker_style),
                    Span::styled(format!("{} ", indicator), indicator_style),
                    Span::styled(&session.nickname, name_style),
                    Span::styled(" ", name_style),
                    Span::styled(format!("[{}]", label), label_style),
                ]);

                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(block);
        Widget::render(list, area, buf);
    }
}

fn status_indicator(status: &SessionStatus) -> (&'static str, Color) {
    match status {
        SessionStatus::Starting => ("○", Color::DarkGray),
        SessionStatus::Running { .. } => ("◐", Color::Yellow),
        SessionStatus::NeedsAttention { reason, .. } => match reason {
            AttentionReason::ToolError { .. } => ("●", Color::Red),
            AttentionReason::PermissionPrompt { .. } => ("●", Color::Red),
            AttentionReason::Completed => ("✓", Color::Green),
        },
        SessionStatus::Stopped { .. } => ("✓", Color::Green),
        SessionStatus::Error { .. } => ("✗", Color::Red),
    }
}

fn status_label_style(status: &SessionStatus) -> Style {
    match status {
        SessionStatus::NeedsAttention { reason, .. } => match reason {
            AttentionReason::ToolError { .. } => Style::default().fg(Color::Red),
            AttentionReason::PermissionPrompt { .. } => {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            }
            AttentionReason::Completed => Style::default().fg(Color::Green),
        },
        SessionStatus::Running { .. } => Style::default().fg(Color::Yellow),
        SessionStatus::Error { .. } => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::DarkGray),
    }
}
