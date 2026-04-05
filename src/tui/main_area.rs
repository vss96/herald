use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Widget};

use crate::session::terminal::TerminalBuffer;

/// Main area widget that renders a terminal buffer.
pub struct MainArea<'a> {
    terminal: Option<&'a TerminalBuffer>,
    title: String,
}

impl<'a> MainArea<'a> {
    pub fn new(terminal: Option<&'a TerminalBuffer>, title: String) -> Self {
        Self { terminal, title }
    }
}

impl<'a> Widget for MainArea<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        Widget::render(block, area, buf);

        let Some(terminal) = self.terminal else {
            // No active session — show welcome message
            let msg = "Press 'n' to create a new session";
            if inner.width as usize > msg.len() && inner.height > 1 {
                let x = inner.x + (inner.width - msg.len() as u16) / 2;
                let y = inner.y + inner.height / 2;
                buf.set_string(x, y, msg, Style::default().fg(Color::DarkGray));
            }
            return;
        };

        // Render terminal buffer cells into the ratatui buffer
        let render_rows = inner.height.min(terminal.rows());
        let render_cols = inner.width.min(terminal.cols());

        for row in 0..render_rows {
            for col in 0..render_cols {
                let cell = terminal.cell(row, col);
                let buf_x = inner.x + col;
                let buf_y = inner.y + row;
                if let Some(buf_cell) = buf.cell_mut((buf_x, buf_y)) {
                    buf_cell.set_char(cell.ch);
                    buf_cell.set_style(cell.style);
                }
            }
        }
    }
}
