use ansi_to_tui::IntoText;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

const HERALD_LOGO: &str = r#"
        /\  ||  /\
       /  \ || /  \
      / /\ \||/ /\ \
     |  \/  \/  \/  |
      \   .-""-. ,/
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
"#;

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
            let logo_lines: Vec<&str> = HERALD_LOGO
                .lines()
                .filter(|l| !l.is_empty())
                .collect();
            let logo_height = logo_lines.len() as u16;
            let logo_width = logo_lines.iter().map(|l| l.len()).max().unwrap_or(0);

            // If terminal is big enough, show logo + help text
            let total_height = logo_height + 2; // logo + gap + help msg
            if inner.height >= total_height && inner.width as usize >= logo_width {
                let start_y = inner.y + (inner.height.saturating_sub(total_height)) / 2;
                for (i, line) in logo_lines.iter().enumerate() {
                    let x = inner.x + inner.width.saturating_sub(line.len() as u16) / 2;
                    buf.set_string(x, start_y + i as u16, line, Style::default().fg(Color::Cyan));
                }
                let msg_y = start_y + logo_height + 1;
                let msg_x = inner.x + inner.width.saturating_sub(msg.len() as u16) / 2;
                buf.set_string(msg_x, msg_y, msg, Style::default().fg(Color::DarkGray));
            } else if inner.width as usize > msg.len() && inner.height > 1 {
                // Fallback: just show help text if too small for logo
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_helpers::render_to_string;

    #[test]
    fn main_area_empty_large() {
        let widget = MainArea::new(None, "herald".to_string());
        let output = render_to_string(widget, 60, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn main_area_empty_small() {
        let widget = MainArea::new(None, "herald".to_string());
        let output = render_to_string(widget, 40, 5);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn main_area_with_plain_text() {
        let content = "hello world\nline 2\nline 3".to_string();
        let widget = MainArea::new(Some(content), "test-session".to_string());
        let output = render_to_string(widget, 60, 10);
        insta::assert_snapshot!(output);
    }
}
