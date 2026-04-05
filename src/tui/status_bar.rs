use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

const BG: Color = Color::Rgb(30, 30, 46);

pub struct StatusBar<'a> {
    focus_label: &'a str,
    session_count: usize,
    attention_count: usize,
}

impl<'a> StatusBar<'a> {
    pub fn new(focus_label: &'a str, session_count: usize, attention_count: usize) -> Self {
        Self { focus_label, session_count, attention_count }
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        buf.set_style(area, Style::default().bg(BG));

        let attention_span = if self.attention_count > 0 {
            Span::styled(
                format!("{} need attention", self.attention_count),
                Style::default().fg(Color::Red).bg(BG).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("all clear", Style::default().fg(Color::Green).bg(BG))
        };

        let line = Line::from(vec![
            Span::styled(" herald ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} ", self.focus_label), Style::default().fg(Color::Cyan).bg(BG)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray).bg(BG)),
            Span::styled(format!("{} sessions", self.session_count), Style::default().fg(Color::White).bg(BG)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray).bg(BG)),
            attention_span,
            Span::styled(" | ", Style::default().fg(Color::DarkGray).bg(BG)),
            Span::styled("q:quit n:new x:kill C-g:sidebar", Style::default().fg(Color::DarkGray).bg(BG)),
        ]);

        buf.set_line(area.x, area.y, &line, area.width);
    }
}
