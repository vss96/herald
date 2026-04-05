use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The resolved tmux action for a given key event.
#[derive(Debug, PartialEq)]
pub enum TmuxKey {
    /// Send as a literal string via `send-keys -l`.
    Literal(String),
    /// Send as a named special key via `send-keys`.
    Special(String),
    /// No tmux action (key is consumed locally or ignored).
    Ignored,
}

/// Map a crossterm `KeyEvent` to a `TmuxKey` action.
///
/// This is a pure function — it has no side effects and does not spawn tasks.
pub fn map_key(key: KeyEvent) -> TmuxKey {
    match key.code {
        KeyCode::Esc => TmuxKey::Special("Escape".to_string()),
        KeyCode::Enter => TmuxKey::Special("Enter".to_string()),
        KeyCode::Backspace => TmuxKey::Special("BSpace".to_string()),
        KeyCode::Up => TmuxKey::Special("Up".to_string()),
        KeyCode::Down => TmuxKey::Special("Down".to_string()),
        KeyCode::Left => TmuxKey::Special("Left".to_string()),
        KeyCode::Right => TmuxKey::Special("Right".to_string()),
        KeyCode::BackTab => TmuxKey::Special("BTab".to_string()),
        KeyCode::Tab => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                TmuxKey::Special("BTab".to_string())
            } else {
                TmuxKey::Special("Tab".to_string())
            }
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::SHIFT | KeyModifiers::CONTROL) {
                TmuxKey::Literal(format!("C-S-{}", c))
            } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                TmuxKey::Literal(format!("C-{}", c))
            } else {
                // Plain char or Shift+char — send the char as-is
                // (uppercase is already encoded in `c` by crossterm for Shift+char)
                TmuxKey::Literal(c.to_string())
            }
        }
        _ => TmuxKey::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn plain(code: KeyCode) -> KeyEvent {
        key(code, KeyModifiers::NONE)
    }

    #[test]
    fn esc_forwards_to_tmux() {
        assert_eq!(map_key(plain(KeyCode::Esc)), TmuxKey::Special("Escape".to_string()));
    }

    #[test]
    fn enter_maps_to_special_enter() {
        assert_eq!(map_key(plain(KeyCode::Enter)), TmuxKey::Special("Enter".to_string()));
    }

    #[test]
    fn backspace_maps_to_special_bspace() {
        assert_eq!(map_key(plain(KeyCode::Backspace)), TmuxKey::Special("BSpace".to_string()));
    }

    #[test]
    fn arrow_keys_map_to_specials() {
        assert_eq!(map_key(plain(KeyCode::Up)), TmuxKey::Special("Up".to_string()));
        assert_eq!(map_key(plain(KeyCode::Down)), TmuxKey::Special("Down".to_string()));
        assert_eq!(map_key(plain(KeyCode::Left)), TmuxKey::Special("Left".to_string()));
        assert_eq!(map_key(plain(KeyCode::Right)), TmuxKey::Special("Right".to_string()));
    }

    #[test]
    fn backtab_keycode_maps_to_btab() {
        assert_eq!(map_key(plain(KeyCode::BackTab)), TmuxKey::Special("BTab".to_string()));
    }

    #[test]
    fn tab_with_shift_modifier_maps_to_btab() {
        assert_eq!(
            map_key(key(KeyCode::Tab, KeyModifiers::SHIFT)),
            TmuxKey::Special("BTab".to_string())
        );
    }

    #[test]
    fn tab_without_shift_maps_to_tab() {
        assert_eq!(map_key(plain(KeyCode::Tab)), TmuxKey::Special("Tab".to_string()));
    }

    #[test]
    fn ctrl_shift_char_maps_to_literal_c_s() {
        assert_eq!(
            map_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)),
            TmuxKey::Literal("C-S-a".to_string())
        );
    }

    #[test]
    fn ctrl_char_maps_to_literal_c() {
        assert_eq!(
            map_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            TmuxKey::Literal("C-c".to_string())
        );
    }

    #[test]
    fn plain_char_maps_to_literal() {
        assert_eq!(
            map_key(plain(KeyCode::Char('x'))),
            TmuxKey::Literal("x".to_string())
        );
    }

    #[test]
    fn shift_char_maps_to_literal() {
        // Crossterm encodes Shift+a as Char('A'), so we just send the char as-is
        assert_eq!(
            map_key(key(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            TmuxKey::Literal("A".to_string())
        );
    }

    #[test]
    fn unknown_key_is_ignored() {
        assert_eq!(map_key(plain(KeyCode::F(1))), TmuxKey::Ignored);
        assert_eq!(map_key(plain(KeyCode::Insert)), TmuxKey::Ignored);
    }
}
