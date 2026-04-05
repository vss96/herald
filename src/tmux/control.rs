/// tmux control mode (-CC) event parser and client.
///
/// Control mode sends events as lines on stdout:
///   %output %<pane-id> <octal-escaped-data>
///   %begin <time> <cmd-number> <flags>
///   <response lines...>
///   %end <time> <cmd-number> <flags>
///   %window-add @<window-id>
///   %window-close @<window-id>
///   %session-changed $<session-id> <name>
///   %sessions-changed
///
/// Characters < ASCII 32 and backslash are octal-escaped in %output data.

#[derive(Debug, Clone, PartialEq)]
pub enum ControlEvent {
    Output { pane_id: String, data: Vec<u8> },
    CommandDone { cmd_number: u64, body: String },
    CommandError { cmd_number: u64, body: String },
    WindowAdd { window_id: String },
    WindowClose { window_id: String },
    SessionChanged { session_id: String, name: String },
    SessionsChanged,
}

/// Decode tmux octal-escaped output data.
///
/// Characters < ASCII 32 and backslash are replaced with \NNN (3-digit octal).
pub fn decode_octal_escapes(input: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            // Try to parse 3 octal digits
            let d1 = bytes[i + 1];
            let d2 = bytes[i + 2];
            let d3 = bytes[i + 3];
            if (b'0'..=b'7').contains(&d1)
                && (b'0'..=b'7').contains(&d2)
                && (b'0'..=b'7').contains(&d3)
            {
                let val = (d1 - b'0') as u16 * 64 + (d2 - b'0') as u16 * 8 + (d3 - b'0') as u16;
                result.push(val as u8);
                i += 4;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }

    result
}

/// Parse a single line from tmux control mode stdout into an event.
///
/// Returns None for lines that aren't recognized single-line events.
/// Use `ControlParser` for stateful multi-line parsing.
pub fn parse_control_line(line: &str) -> Option<ControlEvent> {
    if let Some(rest) = line.strip_prefix("%output ") {
        let space = rest.find(' ')?;
        let pane_id = rest[..space].to_string();
        let data = decode_octal_escapes(&rest[space + 1..]);
        return Some(ControlEvent::Output { pane_id, data });
    }

    if let Some(rest) = line.strip_prefix("%window-add ") {
        return Some(ControlEvent::WindowAdd {
            window_id: rest.to_string(),
        });
    }

    if let Some(rest) = line.strip_prefix("%window-close ") {
        return Some(ControlEvent::WindowClose {
            window_id: rest.to_string(),
        });
    }

    if let Some(rest) = line.strip_prefix("%session-changed ") {
        let space = rest.find(' ')?;
        return Some(ControlEvent::SessionChanged {
            session_id: rest[..space].to_string(),
            name: rest[space + 1..].to_string(),
        });
    }

    if line == "%sessions-changed" {
        return Some(ControlEvent::SessionsChanged);
    }

    None
}

/// Stateful parser for tmux control mode output.
///
/// Handles multi-line command responses (between %begin and %end/%error guards).
pub struct ControlParser {
    pending_cmd: Option<PendingCommand>,
}

struct PendingCommand {
    cmd_number: u64,
    lines: Vec<String>,
}

impl ControlParser {
    pub fn new() -> Self {
        Self { pending_cmd: None }
    }

    /// Feed a line of control mode output. Returns an event if one is complete.
    pub fn feed_line(&mut self, line: &str) -> Option<ControlEvent> {
        // %output events can arrive at any time, even during a %begin block
        if line.starts_with("%output ") {
            return parse_control_line(line);
        }

        // Check for %begin
        if line.starts_with("%begin ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let cmd_number = parts[2].parse::<u64>().unwrap_or(0);
                self.pending_cmd = Some(PendingCommand {
                    cmd_number,
                    lines: Vec::new(),
                });
            }
            return None;
        }

        // Check for %end
        if line.starts_with("%end ") {
            if let Some(pending) = self.pending_cmd.take() {
                return Some(ControlEvent::CommandDone {
                    cmd_number: pending.cmd_number,
                    body: pending.lines.join("\n"),
                });
            }
            return None;
        }

        // Check for %error
        if line.starts_with("%error ") {
            if let Some(pending) = self.pending_cmd.take() {
                return Some(ControlEvent::CommandError {
                    cmd_number: pending.cmd_number,
                    body: pending.lines.join("\n"),
                });
            }
            return None;
        }

        // If we're inside a %begin block, accumulate lines
        if let Some(ref mut pending) = self.pending_cmd {
            pending.lines.push(line.to_string());
            return None;
        }

        // Otherwise, try single-line parse
        parse_control_line(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── decode_octal_escapes ──

    #[test]
    fn decode_plain_ascii() {
        assert_eq!(decode_octal_escapes("hello world"), b"hello world");
    }

    #[test]
    fn decode_newline_octal() {
        assert_eq!(decode_octal_escapes(r"line1\012line2"), b"line1\nline2");
    }

    #[test]
    fn decode_backslash_octal() {
        assert_eq!(decode_octal_escapes(r"path\134to\134file"), b"path\\to\\file");
    }

    #[test]
    fn decode_tab_octal() {
        assert_eq!(decode_octal_escapes(r"col1\011col2"), b"col1\tcol2");
    }

    #[test]
    fn decode_mixed_escapes_and_plain() {
        assert_eq!(
            decode_octal_escapes(r"$ echo hi\012hi\012"),
            b"$ echo hi\nhi\n"
        );
    }

    #[test]
    fn decode_empty_string() {
        assert_eq!(decode_octal_escapes(""), b"");
    }

    #[test]
    fn decode_rejects_non_octal_digits() {
        // \890 contains digits 8 and 9 which are not valid octal — should pass through as literal
        assert_eq!(decode_octal_escapes(r"\890"), br"\890");
    }

    // ── parse_control_line ──

    #[test]
    fn parse_output_event() {
        let event = parse_control_line(r"%output %0 hello\012").unwrap();
        assert_eq!(
            event,
            ControlEvent::Output {
                pane_id: "%0".to_string(),
                data: b"hello\n".to_vec(),
            }
        );
    }

    #[test]
    fn parse_window_add() {
        let event = parse_control_line("%window-add @1").unwrap();
        assert_eq!(
            event,
            ControlEvent::WindowAdd {
                window_id: "@1".to_string()
            }
        );
    }

    #[test]
    fn parse_window_close() {
        let event = parse_control_line("%window-close @2").unwrap();
        assert_eq!(
            event,
            ControlEvent::WindowClose {
                window_id: "@2".to_string()
            }
        );
    }

    #[test]
    fn parse_session_changed() {
        let event = parse_control_line("%session-changed $1 my-session").unwrap();
        assert_eq!(
            event,
            ControlEvent::SessionChanged {
                session_id: "$1".to_string(),
                name: "my-session".to_string(),
            }
        );
    }

    #[test]
    fn parse_sessions_changed() {
        let event = parse_control_line("%sessions-changed").unwrap();
        assert_eq!(event, ControlEvent::SessionsChanged);
    }

    #[test]
    fn parse_unknown_line_returns_none() {
        assert!(parse_control_line("some random output").is_none());
    }

    // ── ControlParser ──

    #[test]
    fn parser_single_line_events_pass_through() {
        let mut parser = ControlParser::new();
        let event = parser.feed_line(r"%output %0 data\012").unwrap();
        assert_eq!(
            event,
            ControlEvent::Output {
                pane_id: "%0".to_string(),
                data: b"data\n".to_vec(),
            }
        );
    }

    #[test]
    fn parser_command_response_success() {
        let mut parser = ControlParser::new();
        assert!(parser.feed_line("%begin 1234 42 0").is_none());
        assert!(parser.feed_line("session: herald").is_none());
        assert!(parser.feed_line("windows: 3").is_none());
        let event = parser.feed_line("%end 1234 42 0").unwrap();
        assert_eq!(
            event,
            ControlEvent::CommandDone {
                cmd_number: 42,
                body: "session: herald\nwindows: 3".to_string(),
            }
        );
    }

    #[test]
    fn parser_command_response_error() {
        let mut parser = ControlParser::new();
        assert!(parser.feed_line("%begin 1234 7 0").is_none());
        assert!(parser.feed_line("no such session: foo").is_none());
        let event = parser.feed_line("%error 1234 7 0").unwrap();
        assert_eq!(
            event,
            ControlEvent::CommandError {
                cmd_number: 7,
                body: "no such session: foo".to_string(),
            }
        );
    }

    #[test]
    fn parser_interleaved_output_during_command() {
        let mut parser = ControlParser::new();
        assert!(parser.feed_line("%begin 1234 1 0").is_none());
        let event = parser.feed_line(r"%output %0 interleaved\012").unwrap();
        assert_eq!(
            event,
            ControlEvent::Output {
                pane_id: "%0".to_string(),
                data: b"interleaved\n".to_vec(),
            }
        );
        assert!(parser.feed_line("response body").is_none());
        let event = parser.feed_line("%end 1234 1 0").unwrap();
        assert_eq!(
            event,
            ControlEvent::CommandDone {
                cmd_number: 1,
                body: "response body".to_string(),
            }
        );
    }
}
