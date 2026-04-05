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
}
