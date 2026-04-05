/// Input state for the new-session dialog.
#[derive(Debug, Default)]
pub struct NewSessionDialog {
    pub nickname: String,
    pub prompt: String,
    pub working_dir: String,
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

    pub fn active_input_mut(&mut self) -> &mut String {
        match self.active_field {
            DialogField::Nickname => &mut self.nickname,
            DialogField::WorkingDir => &mut self.working_dir,
            DialogField::Prompt => &mut self.prompt,
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.nickname.trim().is_empty() && !self.prompt.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        d.nickname = "test".into();
        assert!(!d.is_valid());
        d.prompt = "fix tests".into();
        assert!(d.is_valid());
    }

    #[test]
    fn reset_clears_all() {
        let mut d = NewSessionDialog {
            nickname: "test".into(),
            prompt: "prompt".into(),
            working_dir: "/tmp".into(),
            active_field: DialogField::Prompt,
            visible: true,
        };
        d.reset();
        assert!(d.nickname.is_empty());
        assert!(d.prompt.is_empty());
        assert!(!d.visible);
        assert_eq!(d.active_field, DialogField::Nickname);
    }
}
