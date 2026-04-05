use std::path::PathBuf;
use std::time::Instant;

use crate::session::terminal::TerminalBuffer;

/// Status of a Claude Code session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    Starting,
    Running { last_activity: Instant },
    NeedsAttention { reason: AttentionReason, since: Instant },
    Stopped { exit_code: Option<i32> },
    Error { message: String },
}

/// Why a session needs human attention.
#[derive(Debug, Clone, PartialEq)]
pub enum AttentionReason {
    PermissionPrompt {
        tool_name: String,
        tool_use_id: Option<String>,
    },
    ToolError {
        tool_name: String,
        error: String,
    },
    Completed,
}

/// A managed Claude Code session.
pub struct Session {
    pub id: String,
    pub nickname: String,
    pub tmux_pane_id: String,
    pub prompt: String,
    pub working_dir: PathBuf,
    pub status: SessionStatus,
    pub created_at: Instant,
    pub terminal: TerminalBuffer,
}

impl Session {
    pub fn new(
        id: String,
        nickname: String,
        prompt: String,
        working_dir: PathBuf,
        terminal_cols: u16,
        terminal_rows: u16,
    ) -> Self {
        Self {
            id,
            nickname,
            tmux_pane_id: String::new(),
            prompt,
            working_dir,
            status: SessionStatus::Starting,
            created_at: Instant::now(),
            terminal: TerminalBuffer::new(terminal_cols, terminal_rows),
        }
    }

    pub fn is_alive(&self) -> bool {
        !matches!(
            self.status,
            SessionStatus::Stopped { .. } | SessionStatus::Error { .. }
        )
    }

    /// Short status label for the sidebar.
    pub fn status_label(&self) -> &str {
        match &self.status {
            SessionStatus::Starting => "starting",
            SessionStatus::Running { .. } => "running",
            SessionStatus::NeedsAttention { reason, .. } => match reason {
                AttentionReason::PermissionPrompt { .. } => "needs input",
                AttentionReason::ToolError { .. } => "error",
                AttentionReason::Completed => "done",
            },
            SessionStatus::Stopped { .. } => "stopped",
            SessionStatus::Error { .. } => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_starts_in_starting_state() {
        let s = Session::new(
            "id1".into(),
            "test".into(),
            "fix tests".into(),
            PathBuf::from("/tmp"),
            80,
            24,
        );
        assert!(matches!(s.status, SessionStatus::Starting));
        assert!(s.is_alive());
    }

    #[test]
    fn stopped_session_is_not_alive() {
        let mut s = Session::new(
            "id1".into(),
            "test".into(),
            "fix tests".into(),
            PathBuf::from("/tmp"),
            80,
            24,
        );
        s.status = SessionStatus::Stopped { exit_code: Some(0) };
        assert!(!s.is_alive());
    }

    #[test]
    fn status_labels() {
        let mut s = Session::new(
            "id1".into(),
            "test".into(),
            "prompt".into(),
            PathBuf::from("/tmp"),
            80,
            24,
        );
        assert_eq!(s.status_label(), "starting");

        s.status = SessionStatus::Running {
            last_activity: Instant::now(),
        };
        assert_eq!(s.status_label(), "running");

        s.status = SessionStatus::NeedsAttention {
            reason: AttentionReason::PermissionPrompt {
                tool_name: "Edit".into(),
                tool_use_id: None,
            },
            since: Instant::now(),
        };
        assert_eq!(s.status_label(), "needs input");

        s.status = SessionStatus::NeedsAttention {
            reason: AttentionReason::ToolError {
                tool_name: "Bash".into(),
                error: "fail".into(),
            },
            since: Instant::now(),
        };
        assert_eq!(s.status_label(), "error");

        s.status = SessionStatus::NeedsAttention {
            reason: AttentionReason::Completed,
            since: Instant::now(),
        };
        assert_eq!(s.status_label(), "done");
    }
}
