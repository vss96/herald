use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget, Widget};

use crate::session::model::{Session, SessionStatus, AttentionReason};

/// Sidebar widget showing all sessions with status.
pub struct Sidebar<'a> {
    sessions: &'a [&'a Session],
    active_session_id: Option<&'a str>,
}

impl<'a> Sidebar<'a> {
    pub fn new(sessions: &'a [&'a Session], active_session_id: Option<&'a str>) -> Self {
        Self {
            sessions,
            active_session_id,
        }
    }
}

impl<'a> Widget for Sidebar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = format!(" Sessions ({}) ", self.sessions.len());
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .map(|session| {
                let is_active = self
                    .active_session_id
                    .map_or(false, |id| id == session.id);

                let (indicator, indicator_color) = status_indicator(&session.status);
                let name_style = if is_active {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let label = session.status_label();
                let label_style = status_label_style(&session.status);

                let line = Line::from(vec![
                    Span::styled(format!(" {} ", indicator), Style::default().fg(indicator_color)),
                    Span::styled(&session.nickname, name_style),
                    Span::raw(" "),
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
