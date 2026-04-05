use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Split the terminal into main area (left) and sidebar (right).
pub fn split_main_sidebar(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(70),
            Constraint::Percentage(30),
        ])
        .split(area);
    (chunks[0], chunks[1])
}

/// Split the main area into content (top) and status bar (bottom).
pub fn split_content_status(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    (chunks[0], chunks[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_splits_correctly() {
        let area = Rect::new(0, 0, 100, 40);
        let (main, sidebar) = split_main_sidebar(area);
        assert_eq!(main.width, 70);
        assert_eq!(sidebar.width, 30);
        assert_eq!(main.height, 40);
    }

    #[test]
    fn status_bar_is_one_line() {
        let area = Rect::new(0, 0, 80, 24);
        let (content, status) = split_content_status(area);
        assert_eq!(status.height, 1);
        assert_eq!(content.height, 23);
    }
}
