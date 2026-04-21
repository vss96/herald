use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

// ── TOML deserialization types ──────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub keys: KeysSection,
    #[serde(default)]
    pub providers: ProvidersConfig,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProvidersConfig {
    /// Default provider ID (e.g., "claude-code"). Used as the initial
    /// selection in the new session dialog.
    pub default: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct KeysSection {
    #[serde(default)]
    pub sidebar: SidebarSection,
    #[serde(default)]
    pub main_area: MainAreaSection,
    #[serde(default)]
    pub dialog: DialogSection,
}

#[derive(Debug, Default, Deserialize)]
pub struct SidebarSection {
    pub quit: Option<OneOrMany>,
    pub new_session: Option<OneOrMany>,
    pub move_down: Option<OneOrMany>,
    pub move_up: Option<OneOrMany>,
    pub select_session: Option<OneOrMany>,
    pub switch_to_main: Option<OneOrMany>,
    pub dismiss: Option<OneOrMany>,
    pub kill_session: Option<OneOrMany>,
    pub force_quit: Option<OneOrMany>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MainAreaSection {
    pub return_to_sidebar: Option<OneOrMany>,
    pub scroll_up: Option<OneOrMany>,
    pub scroll_down: Option<OneOrMany>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DialogSection {
    pub close: Option<OneOrMany>,
    pub next_field: Option<OneOrMany>,
    pub submit: Option<OneOrMany>,
}

/// Accepts either a single string or an array of strings in TOML.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_strings(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

// ── Runtime keybinding types ────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct KeyBindings {
    pub sidebar: SidebarKeys,
    pub main_area: MainAreaKeys,
    pub dialog: DialogKeys,
}

#[derive(Debug, Clone)]
pub struct SidebarKeys {
    pub quit: Vec<KeyEvent>,
    pub new_session: Vec<KeyEvent>,
    pub move_down: Vec<KeyEvent>,
    pub move_up: Vec<KeyEvent>,
    pub select_session: Vec<KeyEvent>,
    pub switch_to_main: Vec<KeyEvent>,
    pub dismiss: Vec<KeyEvent>,
    pub kill_session: Vec<KeyEvent>,
    pub force_quit: Vec<KeyEvent>,
}

#[derive(Debug, Clone)]
pub struct MainAreaKeys {
    pub return_to_sidebar: Vec<KeyEvent>,
    pub scroll_up: Vec<KeyEvent>,
    pub scroll_down: Vec<KeyEvent>,
}

#[derive(Debug, Clone)]
pub struct DialogKeys {
    pub close: Vec<KeyEvent>,
    pub next_field: Vec<KeyEvent>,
    pub submit: Vec<KeyEvent>,
}

// ── Defaults (match current hardcoded behavior) ─────────────────────

impl Default for SidebarKeys {
    fn default() -> Self {
        Self {
            quit: vec![KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)],
            new_session: vec![KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)],
            move_down: vec![
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            ],
            move_up: vec![
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            ],
            select_session: vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
            switch_to_main: vec![KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)],
            dismiss: vec![KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)],
            kill_session: vec![
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
            ],
            force_quit: vec![KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)],
        }
    }
}

impl Default for MainAreaKeys {
    fn default() -> Self {
        Self {
            return_to_sidebar: vec![KeyEvent::new(
                KeyCode::Char('g'),
                KeyModifiers::CONTROL,
            )],
            scroll_up: vec![
                KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT),
            ],
            scroll_down: vec![
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::SHIFT),
            ],
        }
    }
}

impl Default for DialogKeys {
    fn default() -> Self {
        Self {
            close: vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            next_field: vec![KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)],
            submit: vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
        }
    }
}

// ── Key parsing ─────────────────────────────────────────────────────

/// Parse a key string like "ctrl+g", "shift+tab", "q" into a KeyEvent.
pub fn parse_key(s: &str) -> Result<KeyEvent, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty key string".to_string());
    }

    let parts: Vec<&str> = s.split('+').collect();
    let mut modifiers = KeyModifiers::NONE;

    for &part in &parts[..parts.len() - 1] {
        match part.to_lowercase().as_str() {
            "ctrl" | "c" => modifiers |= KeyModifiers::CONTROL,
            "shift" | "s" => modifiers |= KeyModifiers::SHIFT,
            "alt" | "a" => modifiers |= KeyModifiers::ALT,
            other => return Err(format!("unknown modifier: {}", other)),
        }
    }

    let key_part = parts.last().unwrap();
    let code = match key_part.to_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backspace" | "bspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdown" | "pgdn" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        _ if key_part.len() == 1 => KeyCode::Char(key_part.chars().next().unwrap()),
        other => return Err(format!("unknown key: {}", other)),
    };

    Ok(KeyEvent::new(code, modifiers))
}

/// Format a KeyEvent for display (e.g., in status bar hints).
pub fn format_key(key: &KeyEvent) -> String {
    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("C");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("A");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("S");
    }

    let key_name = match key.code {
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Backspace => "Bksp".to_string(),
        KeyCode::Delete => "Del".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PgUp".to_string(),
        KeyCode::PageDown => "PgDn".to_string(),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        _ => "?".to_string(),
    };

    parts.push(&key_name);
    parts.join("-")
}

/// Check if a key event matches any of the configured bindings.
pub fn key_matches(key: &KeyEvent, bindings: &[KeyEvent]) -> bool {
    bindings
        .iter()
        .any(|b| b.code == key.code && key.modifiers.contains(b.modifiers))
}

// ── Config loading ──────────────────────────────────────────────────

/// Parse a list of key strings, logging warnings for invalid ones.
fn parse_keys(strings: Vec<String>) -> Vec<KeyEvent> {
    strings
        .into_iter()
        .filter_map(|s| match parse_key(&s) {
            Ok(key) => Some(key),
            Err(e) => {
                tracing::warn!("invalid key binding '{}': {}", s, e);
                None
            }
        })
        .collect()
}

/// Parse an optional TOML field, falling back to defaults if absent or all invalid.
fn resolve(field: Option<OneOrMany>, defaults: Vec<KeyEvent>) -> Vec<KeyEvent> {
    match field {
        None => defaults,
        Some(val) => {
            let parsed = parse_keys(val.into_strings());
            if parsed.is_empty() {
                defaults
            } else {
                parsed
            }
        }
    }
}

/// Load full configuration from a TOML file, falling back to defaults.
pub fn load_config(path: &Path) -> (KeyBindings, ProvidersConfig) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (KeyBindings::default(), ProvidersConfig::default()),
    };

    let config: ConfigFile = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("failed to parse {}: {}", path.display(), e);
            return (KeyBindings::default(), ProvidersConfig::default());
        }
    };

    let providers = config.providers;
    let keybindings = resolve_keybindings(config.keys);
    (keybindings, providers)
}

/// Load keybindings from a TOML file, falling back to defaults.
#[allow(dead_code)]
pub fn load(path: &Path) -> KeyBindings {
    load_config(path).0
}

fn resolve_keybindings(keys: KeysSection) -> KeyBindings {
    let defaults = KeyBindings::default();
    let sb = keys.sidebar;
    let ma = keys.main_area;
    let dl = keys.dialog;

    KeyBindings {
        sidebar: SidebarKeys {
            quit: resolve(sb.quit, defaults.sidebar.quit),
            new_session: resolve(sb.new_session, defaults.sidebar.new_session),
            move_down: resolve(sb.move_down, defaults.sidebar.move_down),
            move_up: resolve(sb.move_up, defaults.sidebar.move_up),
            select_session: resolve(sb.select_session, defaults.sidebar.select_session),
            switch_to_main: resolve(sb.switch_to_main, defaults.sidebar.switch_to_main),
            dismiss: resolve(sb.dismiss, defaults.sidebar.dismiss),
            kill_session: resolve(sb.kill_session, defaults.sidebar.kill_session),
            force_quit: resolve(sb.force_quit, defaults.sidebar.force_quit),
        },
        main_area: MainAreaKeys {
            return_to_sidebar: resolve(
                ma.return_to_sidebar,
                defaults.main_area.return_to_sidebar,
            ),
            scroll_up: resolve(ma.scroll_up, defaults.main_area.scroll_up),
            scroll_down: resolve(ma.scroll_down, defaults.main_area.scroll_down),
        },
        dialog: DialogKeys {
            close: resolve(dl.close, defaults.dialog.close),
            next_field: resolve(dl.next_field, defaults.dialog.next_field),
            submit: resolve(dl.submit, defaults.dialog.submit),
        },
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── parse_key ──

    #[test]
    fn parse_plain_char() {
        let key = parse_key("q").unwrap();
        assert_eq!(key.code, KeyCode::Char('q'));
        assert_eq!(key.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!(parse_key("enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key("esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("tab").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_key("backspace").unwrap().code, KeyCode::Backspace);
        assert_eq!(parse_key("delete").unwrap().code, KeyCode::Delete);
        assert_eq!(parse_key("up").unwrap().code, KeyCode::Up);
        assert_eq!(parse_key("down").unwrap().code, KeyCode::Down);
        assert_eq!(parse_key("space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(parse_key("home").unwrap().code, KeyCode::Home);
        assert_eq!(parse_key("end").unwrap().code, KeyCode::End);
    }

    #[test]
    fn parse_ctrl_modifier() {
        let key = parse_key("ctrl+g").unwrap();
        assert_eq!(key.code, KeyCode::Char('g'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_ctrl_shift() {
        let key = parse_key("ctrl+shift+a").unwrap();
        assert_eq!(key.code, KeyCode::Char('a'));
        assert!(key.modifiers.contains(KeyModifiers::CONTROL));
        assert!(key.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_modifier_case_insensitive() {
        // Modifier names are case-insensitive
        let k1 = parse_key("Ctrl+g").unwrap();
        let k2 = parse_key("ctrl+g").unwrap();
        assert_eq!(k1.code, k2.code);
        assert_eq!(k1.modifiers, k2.modifiers);
    }

    #[test]
    fn parse_named_key_case_insensitive() {
        let k1 = parse_key("Enter").unwrap();
        let k2 = parse_key("enter").unwrap();
        assert_eq!(k1.code, k2.code);
    }

    #[test]
    fn parse_char_preserves_case() {
        let k1 = parse_key("Q").unwrap();
        let k2 = parse_key("q").unwrap();
        assert_eq!(k1.code, KeyCode::Char('Q'));
        assert_eq!(k2.code, KeyCode::Char('q'));
    }

    #[test]
    fn parse_empty_errors() {
        assert!(parse_key("").is_err());
    }

    #[test]
    fn parse_unknown_key_errors() {
        assert!(parse_key("ctrl+").is_err());
        assert!(parse_key("unknownkey").is_err());
    }

    // ── format_key ──

    #[test]
    fn format_plain_char() {
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(format_key(&key), "q");
    }

    #[test]
    fn format_ctrl_key() {
        let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(format_key(&key), "C-g");
    }

    #[test]
    fn format_named_key() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(format_key(&key), "Enter");
    }

    // ── key_matches ──

    #[test]
    fn matches_exact() {
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let bindings = vec![KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)];
        assert!(key_matches(&key, &bindings));
    }

    #[test]
    fn matches_modifier_superset() {
        // crossterm sometimes sends extra modifiers; contains() handles this
        let key = KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        let bindings = vec![KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)];
        assert!(key_matches(&key, &bindings));
    }

    #[test]
    fn no_match_different_key() {
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let bindings = vec![KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)];
        assert!(!key_matches(&key, &bindings));
    }

    #[test]
    fn no_match_missing_modifier() {
        let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
        let bindings = vec![KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)];
        assert!(!key_matches(&key, &bindings));
    }

    #[test]
    fn matches_any_in_list() {
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let bindings = vec![
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        ];
        assert!(key_matches(&key, &bindings));
    }

    // ── load ──

    #[test]
    fn load_missing_file_returns_defaults() {
        let kb = load(Path::new("/nonexistent/herald.toml"));
        assert_eq!(kb.sidebar.quit.len(), 1);
        assert_eq!(kb.sidebar.quit[0].code, KeyCode::Char('q'));
    }

    #[test]
    fn load_empty_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herald.toml");
        std::fs::write(&path, "").unwrap();
        let kb = load(&path);
        assert_eq!(kb.sidebar.quit[0].code, KeyCode::Char('q'));
        assert_eq!(kb.main_area.return_to_sidebar[0].code, KeyCode::Char('g'));
    }

    #[test]
    fn load_partial_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herald.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[keys.sidebar]").unwrap();
        writeln!(f, "quit = \"Q\"").unwrap();
        writeln!(f, "[keys.main_area]").unwrap();
        writeln!(f, "return_to_sidebar = \"ctrl+b\"").unwrap();

        let kb = load(&path);
        // quit overridden to uppercase Q
        assert_eq!(kb.sidebar.quit[0].code, KeyCode::Char('Q'));
        // return_to_sidebar overridden to ctrl+b
        assert_eq!(kb.main_area.return_to_sidebar[0].code, KeyCode::Char('b'));
        assert!(kb.main_area.return_to_sidebar[0]
            .modifiers
            .contains(KeyModifiers::CONTROL));
        // Other defaults preserved
        assert_eq!(kb.sidebar.new_session[0].code, KeyCode::Char('n'));
    }

    #[test]
    fn load_array_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herald.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[keys.sidebar]").unwrap();
        writeln!(f, "move_down = [\"j\", \"down\", \"ctrl+n\"]").unwrap();

        let kb = load(&path);
        assert_eq!(kb.sidebar.move_down.len(), 3);
        assert_eq!(kb.sidebar.move_down[0].code, KeyCode::Char('j'));
        assert_eq!(kb.sidebar.move_down[1].code, KeyCode::Down);
        assert_eq!(kb.sidebar.move_down[2].code, KeyCode::Char('n'));
        assert!(kb.sidebar.move_down[2]
            .modifiers
            .contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn load_invalid_key_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herald.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[keys.sidebar]").unwrap();
        writeln!(f, "quit = \"not_a_real_key\"").unwrap();

        let kb = load(&path);
        // Invalid key → fallback to default
        assert_eq!(kb.sidebar.quit[0].code, KeyCode::Char('q'));
    }

    #[test]
    fn load_invalid_toml_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herald.toml");
        std::fs::write(&path, "this is not valid toml {{{{").unwrap();
        let kb = load(&path);
        assert_eq!(kb.sidebar.quit[0].code, KeyCode::Char('q'));
    }
}
