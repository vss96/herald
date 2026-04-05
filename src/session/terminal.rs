use std::collections::VecDeque;

use ratatui::style::{Color, Modifier, Style};

/// A single cell in the terminal grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            style: Style::default(),
        }
    }
}

/// A terminal emulator buffer that processes VTE escape sequences.
///
/// Maintains a 2D grid of cells with cursor position and style tracking.
pub struct TerminalBuffer {
    cols: u16,
    rows: u16,
    grid: VecDeque<Vec<Cell>>,
    cursor_row: u16,
    cursor_col: u16,
    current_style: Style,
    parser: vte::Parser,
    /// Scrollback lines above the visible area
    scrollback: VecDeque<Vec<Cell>>,
    max_scrollback: usize,
}

impl TerminalBuffer {
    pub fn new(cols: u16, rows: u16) -> Self {
        let grid: VecDeque<Vec<Cell>> =
            (0..rows).map(|_| vec![Cell::default(); cols as usize]).collect();
        Self {
            cols,
            rows,
            grid,
            cursor_row: 0,
            cursor_col: 0,
            current_style: Style::default(),
            parser: vte::Parser::new(),
            scrollback: VecDeque::new(),
            max_scrollback: 10_000,
        }
    }

    /// Process raw bytes from tmux control mode %output.
    pub fn process(&mut self, data: &[u8]) {
        for &byte in data {
            let mut performer = TermPerformer {
                cols: self.cols,
                rows: self.rows,
                grid: &mut self.grid,
                cursor_row: &mut self.cursor_row,
                cursor_col: &mut self.cursor_col,
                current_style: &mut self.current_style,
                scrollback: &mut self.scrollback,
                max_scrollback: self.max_scrollback,
            };
            self.parser.advance(&mut performer, byte);
        }
    }

    pub fn cursor_row(&self) -> u16 {
        self.cursor_row
    }

    pub fn cursor_col(&self) -> u16 {
        self.cursor_col
    }

    pub fn cell(&self, row: u16, col: u16) -> &Cell {
        &self.grid[row as usize][col as usize]
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn grid(&self) -> &VecDeque<Vec<Cell>> {
        &self.grid
    }

    /// Extract visible text content (for testing).
    pub fn text_content(&self) -> String {
        self.grid
            .iter()
            .map(|row| {
                let line: String = row.iter().map(|c| c.ch).collect();
                line.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Resize the terminal buffer.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        let mut grid: VecDeque<Vec<Cell>> =
            (0..rows).map(|_| vec![Cell::default(); cols as usize]).collect();
        let copy_rows = self.rows.min(rows) as usize;
        let copy_cols = self.cols.min(cols) as usize;
        for r in 0..copy_rows {
            for c in 0..copy_cols {
                grid[r][c] = self.grid[r][c].clone();
            }
        }
        self.grid = grid;
        self.cols = cols;
        self.rows = rows;
        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
    }
}

/// Scroll the grid up by one line, pushing top line to scrollback.
fn scroll_up(
    grid: &mut VecDeque<Vec<Cell>>,
    cols: u16,
    scrollback: &mut VecDeque<Vec<Cell>>,
    max_scrollback: usize,
) {
    if let Some(top_row) = grid.pop_front() {
        scrollback.push_back(top_row);
        if scrollback.len() > max_scrollback {
            scrollback.pop_front();
        }
    }
    grid.push_back(vec![Cell::default(); cols as usize]);
}

/// VTE Perform implementation that writes to our grid.
struct TermPerformer<'a> {
    cols: u16,
    rows: u16,
    grid: &'a mut VecDeque<Vec<Cell>>,
    cursor_row: &'a mut u16,
    cursor_col: &'a mut u16,
    current_style: &'a mut Style,
    scrollback: &'a mut VecDeque<Vec<Cell>>,
    max_scrollback: usize,
}

impl<'a> TermPerformer<'a> {
    fn advance_cursor(&mut self) {
        *self.cursor_col += 1;
        if *self.cursor_col >= self.cols {
            *self.cursor_col = 0;
            if *self.cursor_row + 1 >= self.rows {
                scroll_up(self.grid, self.cols, self.scrollback, self.max_scrollback);
            } else {
                *self.cursor_row += 1;
            }
        }
    }
}

impl<'a> vte::Perform for TermPerformer<'a> {
    fn print(&mut self, ch: char) {
        let r = *self.cursor_row as usize;
        let c = *self.cursor_col as usize;
        if r < self.grid.len() && c < self.grid[r].len() {
            self.grid[r][c] = Cell {
                ch,
                style: *self.current_style,
            };
        }
        self.advance_cursor();
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // Newline (\n) — move cursor down
            0x0A => {
                if *self.cursor_row + 1 >= self.rows {
                    scroll_up(self.grid, self.cols, self.scrollback, self.max_scrollback);
                } else {
                    *self.cursor_row += 1;
                }
            }
            // Carriage return (\r) — move cursor to column 0
            0x0D => {
                *self.cursor_col = 0;
            }
            // Tab (\t) — advance to next tab stop (every 8 columns)
            0x09 => {
                let next_tab = ((*self.cursor_col / 8) + 1) * 8;
                *self.cursor_col = next_tab.min(self.cols - 1);
            }
            // Backspace — move cursor left
            0x08 => {
                if *self.cursor_col > 0 {
                    *self.cursor_col -= 1;
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let params: Vec<u16> = params.iter().map(|p| p[0]).collect();

        match action {
            // Cursor Up (CUU)
            'A' => {
                let n = params.first().copied().unwrap_or(1).max(1);
                *self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            // Cursor Down (CUD)
            'B' => {
                let n = params.first().copied().unwrap_or(1).max(1);
                *self.cursor_row = (*self.cursor_row + n).min(self.rows - 1);
            }
            // Cursor Forward (CUF)
            'C' => {
                let n = params.first().copied().unwrap_or(1).max(1);
                *self.cursor_col = (*self.cursor_col + n).min(self.cols - 1);
            }
            // Cursor Back (CUB)
            'D' => {
                let n = params.first().copied().unwrap_or(1).max(1);
                *self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            // Cursor Position (CUP) — \e[row;colH
            'H' | 'f' => {
                let row = params.first().copied().unwrap_or(1).max(1) - 1;
                let col = params.get(1).copied().unwrap_or(1).max(1) - 1;
                *self.cursor_row = row.min(self.rows - 1);
                *self.cursor_col = col.min(self.cols - 1);
            }
            // Erase in Display (ED)
            'J' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    // Clear from cursor to end of screen
                    0 => {
                        for c in (*self.cursor_col as usize)..self.cols as usize {
                            self.grid[*self.cursor_row as usize][c] = Cell::default();
                        }
                        for r in (*self.cursor_row as usize + 1)..self.rows as usize {
                            for c in 0..self.cols as usize {
                                self.grid[r][c] = Cell::default();
                            }
                        }
                    }
                    // Clear entire screen
                    2 | 3 => {
                        for row in self.grid.iter_mut() {
                            for cell in row.iter_mut() {
                                *cell = Cell::default();
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Erase in Line (EL)
            'K' => {
                let mode = params.first().copied().unwrap_or(0);
                let row = *self.cursor_row as usize;
                match mode {
                    // Clear from cursor to end of line
                    0 => {
                        for c in (*self.cursor_col as usize)..self.cols as usize {
                            self.grid[row][c] = Cell::default();
                        }
                    }
                    // Clear entire line
                    2 => {
                        for c in 0..self.cols as usize {
                            self.grid[row][c] = Cell::default();
                        }
                    }
                    _ => {}
                }
            }
            // SGR (Select Graphic Rendition) — colors and styles
            'm' => {
                self.apply_sgr(&params);
            }
            _ => {}
        }
    }
}

impl<'a> TermPerformer<'a> {
    fn apply_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            *self.current_style = Style::default();
            return;
        }

        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => *self.current_style = Style::default(),
                1 => *self.current_style = self.current_style.add_modifier(Modifier::BOLD),
                3 => *self.current_style = self.current_style.add_modifier(Modifier::ITALIC),
                4 => *self.current_style = self.current_style.add_modifier(Modifier::UNDERLINED),
                7 => *self.current_style = self.current_style.add_modifier(Modifier::REVERSED),
                22 => *self.current_style = self.current_style.remove_modifier(Modifier::BOLD),
                23 => *self.current_style = self.current_style.remove_modifier(Modifier::ITALIC),
                24 => *self.current_style = self.current_style.remove_modifier(Modifier::UNDERLINED),
                27 => *self.current_style = self.current_style.remove_modifier(Modifier::REVERSED),
                // Standard foreground colors
                30 => self.current_style.fg = Some(Color::Black),
                31 => self.current_style.fg = Some(Color::Red),
                32 => self.current_style.fg = Some(Color::Green),
                33 => self.current_style.fg = Some(Color::Yellow),
                34 => self.current_style.fg = Some(Color::Blue),
                35 => self.current_style.fg = Some(Color::Magenta),
                36 => self.current_style.fg = Some(Color::Cyan),
                37 => self.current_style.fg = Some(Color::White),
                39 => self.current_style.fg = None, // Default fg
                // Standard background colors
                40 => self.current_style.bg = Some(Color::Black),
                41 => self.current_style.bg = Some(Color::Red),
                42 => self.current_style.bg = Some(Color::Green),
                43 => self.current_style.bg = Some(Color::Yellow),
                44 => self.current_style.bg = Some(Color::Blue),
                45 => self.current_style.bg = Some(Color::Magenta),
                46 => self.current_style.bg = Some(Color::Cyan),
                47 => self.current_style.bg = Some(Color::White),
                49 => self.current_style.bg = None, // Default bg
                // 256-color mode: \e[38;5;Nm or \e[48;5;Nm
                38 if params.get(i + 1) == Some(&5) => {
                    if let Some(&n) = params.get(i + 2) {
                        self.current_style.fg = Some(Color::Indexed(n as u8));
                        i += 2;
                    }
                }
                48 if params.get(i + 1) == Some(&5) => {
                    if let Some(&n) = params.get(i + 2) {
                        self.current_style.bg = Some(Color::Indexed(n as u8));
                        i += 2;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_blank() {
        let buf = TerminalBuffer::new(80, 24);
        assert_eq!(buf.cursor_row(), 0);
        assert_eq!(buf.cursor_col(), 0);
        assert_eq!(buf.cell(0, 0).ch, ' ');
    }

    #[test]
    fn print_simple_text() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"hello");
        assert_eq!(buf.cell(0, 0).ch, 'h');
        assert_eq!(buf.cell(0, 1).ch, 'e');
        assert_eq!(buf.cell(0, 4).ch, 'o');
        assert_eq!(buf.cursor_col(), 5);
        assert_eq!(buf.cursor_row(), 0);
    }

    #[test]
    fn newline_moves_cursor_down() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"line1\nline2");
        assert_eq!(buf.cursor_row(), 1);
        // After \n cursor stays at same col (no \r)
        assert_eq!(buf.cell(0, 0).ch, 'l');
    }

    #[test]
    fn carriage_return_resets_column() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"hello\rworld");
        // "world" overwrites "hello" starting from col 0
        assert_eq!(buf.cell(0, 0).ch, 'w');
        assert_eq!(buf.cell(0, 4).ch, 'd');
        assert_eq!(buf.cell(0, 5).ch, ' '); // 'hello' was 5 chars, 'world' is 5 chars
    }

    #[test]
    fn crlf_moves_to_next_line_start() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"line1\r\nline2");
        assert_eq!(buf.cell(0, 0).ch, 'l');
        assert_eq!(buf.cell(1, 0).ch, 'l');
        assert_eq!(buf.cursor_row(), 1);
        assert_eq!(buf.cursor_col(), 5);
    }

    #[test]
    fn cursor_position_escape() {
        let mut buf = TerminalBuffer::new(80, 24);
        // \e[5;10H moves cursor to row 5, col 10 (1-indexed)
        buf.process(b"\x1b[5;10Hx");
        assert_eq!(buf.cell(4, 9).ch, 'x'); // 0-indexed
    }

    #[test]
    fn cursor_movement_escapes() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"\x1b[5;5H"); // Row 5, Col 5 (1-indexed) = (4, 4)

        // Cursor Up 2
        buf.process(b"\x1b[2A");
        assert_eq!(buf.cursor_row(), 2);

        // Cursor Down 1
        buf.process(b"\x1b[1B");
        assert_eq!(buf.cursor_row(), 3);

        // Cursor Forward 3
        buf.process(b"\x1b[3C");
        assert_eq!(buf.cursor_col(), 7);

        // Cursor Back 2
        buf.process(b"\x1b[2D");
        assert_eq!(buf.cursor_col(), 5);
    }

    #[test]
    fn erase_to_end_of_line() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"hello world");
        buf.process(b"\x1b[1;6H"); // Move to col 6 (after "hello")
        buf.process(b"\x1b[K"); // Erase to end of line
        assert_eq!(buf.cell(0, 0).ch, 'h');
        assert_eq!(buf.cell(0, 4).ch, 'o');
        assert_eq!(buf.cell(0, 5).ch, ' '); // erased
        assert_eq!(buf.cell(0, 10).ch, ' '); // erased
    }

    #[test]
    fn clear_screen() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"some text on screen");
        buf.process(b"\x1b[2J"); // Clear entire screen
        for c in 0..80 {
            assert_eq!(buf.cell(0, c).ch, ' ');
        }
    }

    #[test]
    fn sgr_bold_color() {
        let mut buf = TerminalBuffer::new(80, 24);
        // Bold + Red foreground
        buf.process(b"\x1b[1;31mhello\x1b[0m");
        let cell = buf.cell(0, 0);
        assert_eq!(cell.ch, 'h');
        assert!(cell.style.add_modifier == Modifier::BOLD);
        assert_eq!(cell.style.fg, Some(Color::Red));

        // After reset
        let cell_after = buf.cell(0, 5);
        assert_eq!(cell_after.style, Style::default());
    }

    #[test]
    fn line_wrap_at_edge() {
        let mut buf = TerminalBuffer::new(5, 3); // Tiny: 5 cols, 3 rows
        buf.process(b"abcdefgh");
        // "abcde" on row 0, "fgh" on row 1
        assert_eq!(buf.cell(0, 0).ch, 'a');
        assert_eq!(buf.cell(0, 4).ch, 'e');
        assert_eq!(buf.cell(1, 0).ch, 'f');
        assert_eq!(buf.cell(1, 2).ch, 'h');
    }

    #[test]
    fn scroll_at_bottom() {
        let mut buf = TerminalBuffer::new(10, 3);
        buf.process(b"row1\r\nrow2\r\nrow3\r\nrow4");
        // After scrolling, row1 should be in scrollback
        assert_eq!(buf.scrollback.len(), 1);
        assert_eq!(buf.cell(0, 0).ch, 'r'); // "row2" is now top
        assert_eq!(buf.cell(2, 0).ch, 'r'); // "row4" is at bottom
    }

    #[test]
    fn text_content_extraction() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"hello\r\nworld");
        let content = buf.text_content();
        assert!(content.starts_with("hello\nworld"));
    }

    #[test]
    fn resize_preserves_content() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"hello");
        buf.resize(40, 12);
        assert_eq!(buf.cols(), 40);
        assert_eq!(buf.rows(), 12);
        assert_eq!(buf.cell(0, 0).ch, 'h');
        assert_eq!(buf.cell(0, 4).ch, 'o');
    }

    #[test]
    fn backspace_moves_cursor_left() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"abc\x08x");
        // Backspace moved from col 3 to col 2, then 'x' overwrites 'c'
        assert_eq!(buf.cell(0, 0).ch, 'a');
        assert_eq!(buf.cell(0, 1).ch, 'b');
        assert_eq!(buf.cell(0, 2).ch, 'x');
    }

    #[test]
    fn tab_advances_to_next_stop() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.process(b"ab\tcd");
        assert_eq!(buf.cell(0, 0).ch, 'a');
        assert_eq!(buf.cell(0, 1).ch, 'b');
        // Tab should advance to col 8
        assert_eq!(buf.cell(0, 8).ch, 'c');
        assert_eq!(buf.cell(0, 9).ch, 'd');
    }
}
