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
                    .map_or(false, |id| id == session.id.as_str());
                let is_selected = i == self.selected_index;

                let (indicator, session_color) = session_style(&session.status);

                // Background for selected item — bright enough to be clearly visible
                let bg = if is_selected && self.has_focus {
                    Some(Color::Rgb(60, 60, 90))
                } else if is_selected {
                    Some(Color::Rgb(50, 50, 70))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_helpers::*;
    use std::time::Instant;

    #[test]
    fn sidebar_empty() {
        let sessions: Vec<&Session> = vec![];
        let widget = Sidebar::new(&sessions, None, 0, true);
        let output = render_to_string(widget, 30, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn sidebar_single_starting() {
        let s = make_test_session("s1", "my-session", SessionStatus::Starting);
        let sessions: Vec<&Session> = vec![&s];
        let widget = Sidebar::new(&sessions, Some("s1"), 0, true);
        let output = render_to_string(widget, 30, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn sidebar_multiple_statuses() {
        let s1 = make_test_session("s1", "starting", SessionStatus::Starting);
        let s2 = running_session("s2", "running");
        let s3 = attention_session("s3", "needs-input", AttentionReason::PermissionPrompt {
            tool_name: "Edit".into(),
            tool_use_id: None,
        });
        let s4 = make_test_session("s4", "stopped", SessionStatus::Stopped { exit_code: Some(0) });
        let s5 = make_test_session("s5", "errored", SessionStatus::Error {
            message: "crash".into(),
        });
        let sessions: Vec<&Session> = vec![&s1, &s2, &s3, &s4, &s5];
        // s2 selected (index 1), s3 active
        let widget = Sidebar::new(&sessions, Some("s3"), 1, true);
        let output = render_to_string(widget, 30, 12);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn sidebar_unfocused() {
        let s1 = running_session("s1", "session-a");
        let s2 = running_session("s2", "session-b");
        let sessions: Vec<&Session> = vec![&s1, &s2];
        let widget = Sidebar::new(&sessions, Some("s1"), 0, false);
        let output = render_to_string(widget, 30, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn sidebar_attention_border_is_red() {
        let s = attention_session("s1", "alert", AttentionReason::ToolError {
            tool_name: "Bash".into(),
            error: "fail".into(),
        });
        let sessions: Vec<&Session> = vec![&s];
        let widget = Sidebar::new(&sessions, None, 0, true);
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        widget.render(area, &mut buf);
        // Border top-left corner should be RED
        let corner = buf.cell((0, 0)).unwrap();
        assert_eq!(corner.fg, RED);
    }
}
