use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

/// A text input field with cursor position.
#[derive(Debug, Default, Clone)]
pub struct TextInput {
    pub text: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.remove(prev);
            self.cursor = prev;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor += self.text[self.cursor..].chars().next().map_or(0, |c| c.len_utf8());
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn set(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Text before cursor, cursor char, text after cursor (for rendering).
    pub fn parts(&self) -> (&str, Option<char>, &str) {
        let before = &self.text[..self.cursor];
        let at_cursor = self.text[self.cursor..].chars().next();
        let after_start = self.cursor + at_cursor.map_or(0, |c| c.len_utf8());
        let after = if after_start <= self.text.len() {
            &self.text[after_start..]
        } else {
            ""
        };
        (before, at_cursor, after)
    }
}

/// Input state for the new-session dialog.
#[derive(Debug, Default)]
pub struct NewSessionDialog {
    pub nickname: TextInput,
    pub prompt: TextInput,
    pub working_dir: TextInput,
    pub active_field: DialogField,
    pub visible: bool,
    /// Formatted key labels for the footer (e.g., "Enter", "Tab", "Esc").
    pub key_labels: DialogKeyLabels,
}

/// Pre-formatted key names for dialog footer help text.
#[derive(Debug, Clone)]
pub struct DialogKeyLabels {
    pub submit: String,
    pub next_field: String,
    pub close: String,
}

impl Default for DialogKeyLabels {
    fn default() -> Self {
        Self {
            submit: "Enter".to_string(),
            next_field: "Tab".to_string(),
            close: "Esc".to_string(),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub enum DialogField {
    #[default]
    Nickname,
    WorkingDir,
    Prompt,
}

impl NewSessionDialog {
    pub fn reset(&mut self) {
        self.nickname.clear();
        self.prompt.clear();
        self.working_dir.clear();
        self.active_field = DialogField::Nickname;
        self.visible = false;
    }

    pub fn next_field(&mut self) {
        self.active_field = match self.active_field {
            DialogField::Nickname => DialogField::WorkingDir,
            DialogField::WorkingDir => DialogField::Prompt,
            DialogField::Prompt => DialogField::Nickname,
        };
    }

    pub fn active_input(&mut self) -> &mut TextInput {
        match self.active_field {
            DialogField::Nickname => &mut self.nickname,
            DialogField::WorkingDir => &mut self.working_dir,
            DialogField::Prompt => &mut self.prompt,
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.nickname.text.trim().is_empty() && !self.prompt.text.trim().is_empty()
    }

    /// Tab-complete directory paths (like a terminal).
    pub fn complete_directory_path(&mut self) {
        let input_text = self.working_dir.text.clone();
        let path = PathBuf::from(&input_text);

        let (search_dir, prefix) = if input_text.ends_with('/') || input_text.ends_with(std::path::MAIN_SEPARATOR) {
            (path.clone(), String::new())
        } else {
            let parent = path.parent().unwrap_or_else(|| std::path::Path::new("/"));
            let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            (parent.to_path_buf(), file_name)
        };

        let Ok(entries) = std::fs::read_dir(&search_dir) else {
            return;
        };

        let mut matches: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|ft| ft.is_dir()))
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') && !prefix.starts_with('.') {
                    return false;
                }
                name.starts_with(&prefix)
            })
            .map(|e| e.path())
            .collect();

        matches.sort();

        if matches.len() == 1 {
            let completed = format!("{}/", matches[0].display());
            self.working_dir.set(completed);
        } else if matches.len() > 1 {
            let names: Vec<String> = matches.iter().map(|p| p.display().to_string()).collect();
            if let Some(common) = longest_common_prefix(&names) {
                self.working_dir.set(common);
            }
        }
    }
}

impl Widget for &NewSessionDialog {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Center the dialog
        let dialog_width = 60u16.min(area.width.saturating_sub(4));
        let dialog_height = 11u16;
        let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
        let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

        // Clear the area behind the dialog
        for row in dialog_area.y..dialog_area.y + dialog_area.height {
            for col in dialog_area.x..dialog_area.x + dialog_area.width {
                if let Some(cell) = buf.cell_mut((col, row)) {
                    cell.set_char(' ');
                    cell.set_style(Style::default());
                }
            }
        }

        // Draw border
        let block = Block::default()
            .title(" New Session ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(dialog_area);
        Widget::render(block, dialog_area, buf);

        let fields: Vec<(&str, &TextInput, bool)> = vec![
            ("Nickname", &self.nickname, self.active_field == DialogField::Nickname),
            ("Directory", &self.working_dir, self.active_field == DialogField::WorkingDir),
            ("Prompt", &self.prompt, self.active_field == DialogField::Prompt),
        ];

        for (i, (label, input, active)) in fields.iter().enumerate() {
            let y = inner.y + (i as u16) * 2;
            if y >= inner.y + inner.height {
                break;
            }

            let label_style = if *active {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            buf.set_string(inner.x, y, format!(" {}:", label), label_style);

            let input_x = inner.x + 1;
            let input_y = y + 1;
            if input_y < inner.y + inner.height {
                let val_style = if *active {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::Gray)
                };
                let cursor_style = Style::default().fg(Color::Black).bg(Color::White);

                // Available character slots for text (after the leading space)
                let field_width = (inner.x + inner.width).saturating_sub(input_x + 1) as usize;

                if *active {
                    let (before, at, after) = input.parts();
                    let before_chars: Vec<char> = before.chars().collect();
                    let cursor_pos = before_chars.len();

                    // Scroll so the cursor stays visible
                    let scroll = if cursor_pos >= field_width {
                        cursor_pos - field_width + 1
                    } else {
                        0
                    };

                    let visible_before: String = before_chars[scroll..cursor_pos].iter().collect();
                    let visible_before_width = visible_before.chars().count();

                    buf.set_string(input_x, input_y, format!(" {}", &visible_before), val_style);
                    let cursor_x = input_x + 1 + visible_before_width as u16;
                    let cursor_ch = at.unwrap_or(' ');
                    buf.set_string(cursor_x, input_y, cursor_ch.to_string(), cursor_style);

                    if let Some(_c) = at {
                        let after_x = cursor_x + 1;
                        let remaining = field_width.saturating_sub(visible_before_width + 1);
                        let visible_after: String = after.chars().take(remaining).collect();
                        buf.set_string(after_x, input_y, &visible_after, val_style);
                    }
                } else {
                    let visible: String = input.text.chars().take(field_width).collect();
                    buf.set_string(input_x, input_y, format!(" {}", &visible), val_style);
                }
            }
        }

        // Footer — context-sensitive help
        let footer_y = dialog_area.y + dialog_area.height - 1;
        if footer_y > dialog_area.y {
            let kl = &self.key_labels;
            let help = if self.active_field == DialogField::WorkingDir {
                format!(" {}:next  {}:complete path  {}:cancel", kl.submit, kl.next_field, kl.close)
            } else if self.active_field == DialogField::Prompt {
                format!(" {}:launch  {}:next field  {}:cancel", kl.submit, kl.next_field, kl.close)
            } else {
                format!(" {}:next  {}:next field  {}:cancel", kl.submit, kl.next_field, kl.close)
            };
            buf.set_string(
                inner.x,
                footer_y - 1,
                help,
                Style::default().fg(Color::DarkGray),
            );
        }
    }
}

/// Find the longest common prefix among a list of strings.
fn longest_common_prefix(strings: &[String]) -> Option<String> {
    if strings.is_empty() {
        return None;
    }
    let first = &strings[0];
    let mut prefix_len = first.len();
    for s in &strings[1..] {
        prefix_len = prefix_len.min(s.len());
        for (i, (a, b)) in first.bytes().zip(s.bytes()).enumerate() {
            if a != b {
                prefix_len = prefix_len.min(i);
                break;
            }
        }
    }
    if prefix_len == 0 {
        None
    } else {
        Some(first[..prefix_len].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_input_insert_and_cursor() {
        let mut input = TextInput::default();
        input.insert('h');
        input.insert('i');
        assert_eq!(input.text, "hi");
        assert_eq!(input.cursor, 2);

        input.move_left();
        assert_eq!(input.cursor, 1);

        input.insert('!');
        assert_eq!(input.text, "h!i");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn text_input_backspace() {
        let mut input = TextInput::default();
        input.set("hello".into());
        input.backspace();
        assert_eq!(input.text, "hell");
        input.move_left();
        input.move_left();
        input.backspace();
        assert_eq!(input.text, "hll");
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn text_input_home_end() {
        let mut input = TextInput::default();
        input.set("test".into());
        input.home();
        assert_eq!(input.cursor, 0);
        input.end();
        assert_eq!(input.cursor, 4);
    }

    #[test]
    fn dialog_field_cycling() {
        let mut d = NewSessionDialog::default();
        assert_eq!(d.active_field, DialogField::Nickname);
        d.next_field();
        assert_eq!(d.active_field, DialogField::WorkingDir);
        d.next_field();
        assert_eq!(d.active_field, DialogField::Prompt);
        d.next_field();
        assert_eq!(d.active_field, DialogField::Nickname);
    }

    #[test]
    fn validation() {
        let mut d = NewSessionDialog::default();
        assert!(!d.is_valid());
        d.nickname.set("test".into());
        assert!(!d.is_valid());
        d.prompt.set("fix tests".into());
        assert!(d.is_valid());
    }

    #[test]
    fn reset_clears_all() {
        let mut d = NewSessionDialog::default();
        d.nickname.set("test".into());
        d.prompt.set("prompt".into());
        d.working_dir.set("/tmp".into());
        d.active_field = DialogField::Prompt;
        d.visible = true;
        d.reset();
        assert!(d.nickname.text.is_empty());
        assert!(d.prompt.text.is_empty());
        assert!(!d.visible);
        assert_eq!(d.active_field, DialogField::Nickname);
    }

    #[test]
    fn longest_common_prefix_single_string() {
        let strings = vec!["/usr/local".to_string()];
        assert_eq!(longest_common_prefix(&strings), Some("/usr/local".to_string()));
    }

    #[test]
    fn longest_common_prefix_multiple_with_common() {
        let strings = vec![
            "/usr/local/bin".to_string(),
            "/usr/local/lib".to_string(),
            "/usr/local/share".to_string(),
        ];
        assert_eq!(longest_common_prefix(&strings), Some("/usr/local/".to_string()));
    }

    #[test]
    fn longest_common_prefix_empty_input() {
        let strings: Vec<String> = vec![];
        assert_eq!(longest_common_prefix(&strings), None);
    }

    #[test]
    fn longest_common_prefix_no_common_prefix() {
        let strings = vec!["alpha".to_string(), "beta".to_string()];
        assert_eq!(longest_common_prefix(&strings), None);
    }

    #[test]
    fn dialog_default_state() {
        use crate::tui::test_helpers::buffer_to_string;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let mut dialog = NewSessionDialog::default();
        dialog.visible = true;
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        Widget::render(&dialog, area, &mut buf);
        let output = buffer_to_string(&buf);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn dialog_with_input() {
        use crate::tui::test_helpers::buffer_to_string;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let mut dialog = NewSessionDialog::default();
        dialog.visible = true;
        dialog.nickname.set("my-session".into());
        dialog.working_dir.set("/home/user/project".into());
        dialog.prompt.set("fix the tests".into());
        dialog.active_field = DialogField::Prompt;
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        Widget::render(&dialog, area, &mut buf);
        let output = buffer_to_string(&buf);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn dialog_long_prompt_does_not_overflow() {
        use crate::tui::test_helpers::buffer_to_string;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let mut dialog = NewSessionDialog::default();
        dialog.visible = true;
        dialog.nickname.set("sess".into());
        dialog.prompt.set(
            "if the text is more than a certain set of characters it overflows all the way to the right"
                .into(),
        );
        dialog.active_field = DialogField::Prompt;
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        Widget::render(&dialog, area, &mut buf);
        let output = buffer_to_string(&buf);
        insta::assert_snapshot!(output);
    }
}
