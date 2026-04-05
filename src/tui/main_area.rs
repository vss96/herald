use ansi_to_tui::IntoText;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// Main area widget that renders captured pane content with colors.
pub struct MainArea {
    captured_content: Option<String>,
    title: String,
}

impl MainArea {
    pub fn new(captured_content: Option<String>, title: String) -> Self {
        Self {
            captured_content,
            title,
        }
    }
}

impl Widget for MainArea {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let Some(content) = self.captured_content else {
            let inner = block.inner(area);
            Widget::render(block, area, buf);
            let msg = "Press 'n' to create a new session";
            if inner.width as usize > msg.len() && inner.height > 1 {
                let x = inner.x + (inner.width - msg.len() as u16) / 2;
                let y = inner.y + inner.height / 2;
                buf.set_string(x, y, msg, Style::default().fg(Color::DarkGray));
            }
            return;
        };

        // Parse ANSI escape sequences into styled ratatui Text
        let text = content.into_text().unwrap_or_default();
        let paragraph = Paragraph::new(text).block(block);
        Widget::render(paragraph, area, buf);
    }
}
