use std::borrow::Borrow;
use std::path::PathBuf;
use std::time::Instant;

use serde::Deserialize;

/// Strongly-typed session identifier (UUID string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct SessionId(pub String);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for SessionId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Strongly-typed tmux pane identifier (e.g., "%1").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneId(pub String);

impl std::fmt::Display for PaneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl PaneId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Status of an AI coding session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    Starting,
    Running { last_activity: Instant },
    NeedsAttention { reason: AttentionReason, since: Instant },
    Stopped,
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

/// A managed AI coding session.
pub struct Session {
    pub id: SessionId,
    pub nickname: String,
    pub tmux_pane_id: PaneId,
    pub prompt: String,
    pub working_dir: PathBuf,
    pub provider_id: String,
    pub worktree_path: Option<PathBuf>,
    /// Canonical git toplevel of `working_dir`. Populated when a worktree was
    /// created so that cleanup on kill can call WorktreeManager::remove with
    /// an explicit repo path, rather than inferring from the worktree's
    /// parent chain (which no longer points at the source repo under the
    /// central `<data>/herald/worktrees/` layout).
    pub repo_path: Option<PathBuf>,
    pub status: SessionStatus,
    pub created_at: Instant,
}

impl Session {
    pub fn new(
        id: SessionId,
        nickname: String,
        prompt: String,
        working_dir: PathBuf,
        provider_id: String,
    ) -> Self {
        Self {
            id,
            nickname,
            tmux_pane_id: PaneId(String::new()),
            prompt,
            working_dir,
            provider_id,
            worktree_path: None,
            repo_path: None,
            status: SessionStatus::Starting,
            created_at: Instant::now(),
        }
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
            SessionStatus::Stopped => "stopped",
            SessionStatus::Error { .. } => "error",
        }
    }
}

#[cfg(test)]
impl Session {
    pub fn is_alive(&self) -> bool {
        !matches!(
            self.status,
            SessionStatus::Stopped | SessionStatus::Error { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_starts_in_starting_state() {
        let s = Session::new(
            SessionId("id1".into()),
            "test".into(),
            "fix tests".into(),
            PathBuf::from("/tmp"),
            "claude-code".into(),
        );
        assert!(matches!(s.status, SessionStatus::Starting));
        assert!(s.is_alive());
    }

    #[test]
    fn stopped_session_is_not_alive() {
        let mut s = Session::new(
            SessionId("id1".into()),
            "test".into(),
            "fix tests".into(),
            PathBuf::from("/tmp"),
            "claude-code".into(),
        );
        s.status = SessionStatus::Stopped;
        assert!(!s.is_alive());
    }

    #[test]
    fn status_labels() {
        let mut s = Session::new(
            SessionId("id1".into()),
            "test".into(),
            "prompt".into(),
            PathBuf::from("/tmp"),
            "claude-code".into(),
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
