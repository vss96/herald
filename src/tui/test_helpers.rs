use std::path::PathBuf;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::session::model::{AttentionReason, Session, SessionId, SessionStatus};

/// Convert a ratatui Buffer into a readable string for snapshot testing.
/// Iterates row-by-row, collects cell symbols, trims trailing whitespace,
/// and strips trailing empty lines.
pub fn buffer_to_string(buf: &Buffer) -> String {
    let area = buf.area();
    let mut lines = Vec::new();

    for y in area.y..area.y + area.height {
        let mut line = String::new();
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell((x, y)) {
                let symbol = cell.symbol();
                // Skip empty continuation cells (wide characters)
                if symbol.is_empty() {
                    continue;
                }
                line.push_str(symbol);
            }
        }
        lines.push(line.trim_end().to_string());
    }

    // Strip trailing empty lines
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

/// Render a widget into a Buffer and return the text as a string.
pub fn render_to_string<W: ratatui::widgets::Widget>(widget: W, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    widget.render(area, &mut buf);
    buffer_to_string(&buf)
}

/// Create a test Session with the given status.
pub fn make_test_session(id: &str, nickname: &str, status: SessionStatus) -> Session {
    let mut s = Session::new(
        SessionId(id.to_string()),
        nickname.to_string(),
        "test prompt".to_string(),
        PathBuf::from("/tmp"),
    );
    s.status = status;
    s
}

/// Shortcut to create a Running session.
pub fn running_session(id: &str, nickname: &str) -> Session {
    make_test_session(id, nickname, SessionStatus::Running {
        last_activity: Instant::now(),
    })
}

/// Shortcut to create a NeedsAttention session.
pub fn attention_session(id: &str, nickname: &str, reason: AttentionReason) -> Session {
    make_test_session(id, nickname, SessionStatus::NeedsAttention {
        reason,
        since: Instant::now(),
    })
}
