use serde::Deserialize;

use crate::session::model::SessionId;

/// A hook event received from Claude Code via Unix socket.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct HookEvent {
    pub session_id: SessionId,
    pub hook_event_name: HookEventName,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash)]
pub enum HookEventName {
    PermissionRequest,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Stop,
    Notification,
    SessionStart,
    SessionEnd,
}

/// Priority level for attention queue entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Sidebar-only, no queue entry
    Info = 0,
    /// Session completed
    Low = 1,
    /// Permission prompt awaiting user
    High = 2,
    /// Tool error or crash
    Critical = 3,
}

impl HookEventName {
    /// Map a hook event to its priority level.
    pub fn priority(&self) -> Priority {
        match self {
            Self::PostToolUseFailure => Priority::Critical,
            Self::PermissionRequest => Priority::High,
            Self::Stop => Priority::Low,
            _ => Priority::Info,
        }
    }

    /// Whether this event should create a queue entry.
    pub fn is_queueable(&self) -> bool {
        self.priority() > Priority::Info
    }

    /// Hook events that Herald subscribes to in Claude Code config.
    pub const MANAGED: &[HookEventName] = &[
        HookEventName::PermissionRequest,
        HookEventName::PostToolUse,
        HookEventName::PostToolUseFailure,
        HookEventName::Stop,
        HookEventName::Notification,
    ];

    /// Serde variant name for use in JSON hook configuration.
    pub fn as_config_str(&self) -> &'static str {
        match self {
            Self::PermissionRequest => "PermissionRequest",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::Stop => "Stop",
            Self::Notification => "Notification",
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_permission_request() {
        let json = r#"{
            "session_id": "abc123",
            "hook_event_name": "PermissionRequest",
            "tool_name": "Edit",
            "tool_use_id": "toolu_01ABC",
            "tool_input": {"file_path": "/src/foo.rs"},
            "cwd": "/home/user/project"
        }"#;
        let event: HookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.session_id.as_str(), "abc123");
        assert_eq!(event.hook_event_name, HookEventName::PermissionRequest);
        assert_eq!(event.tool_name.as_deref(), Some("Edit"));
        assert_eq!(event.tool_use_id.as_deref(), Some("toolu_01ABC"));
    }

    #[test]
    fn deserialize_post_tool_use_failure() {
        let json = r#"{
            "session_id": "abc123",
            "hook_event_name": "PostToolUseFailure",
            "tool_name": "Bash"
        }"#;
        let event: HookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.hook_event_name, HookEventName::PostToolUseFailure);
        assert!(event.tool_use_id.is_none());
    }

    #[test]
    fn deserialize_stop_event() {
        let json = r#"{
            "session_id": "abc123",
            "hook_event_name": "Stop"
        }"#;
        let event: HookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.hook_event_name, HookEventName::Stop);
    }

    #[test]
    fn deserialize_notification() {
        let json = r#"{
            "session_id": "abc123",
            "hook_event_name": "Notification"
        }"#;
        let event: HookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.hook_event_name, HookEventName::Notification);
    }

    #[test]
    fn priority_ordering() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Low);
        assert!(Priority::Low > Priority::Info);
    }

    #[test]
    fn event_name_priority_mapping() {
        assert_eq!(HookEventName::PostToolUseFailure.priority(), Priority::Critical);
        assert_eq!(HookEventName::PermissionRequest.priority(), Priority::High);
        assert_eq!(HookEventName::Stop.priority(), Priority::Low);
        assert_eq!(HookEventName::Notification.priority(), Priority::Info);
        assert_eq!(HookEventName::PreToolUse.priority(), Priority::Info);
        assert_eq!(HookEventName::PostToolUse.priority(), Priority::Info);
    }

    #[test]
    fn queueable_events() {
        assert!(HookEventName::PostToolUseFailure.is_queueable());
        assert!(HookEventName::PermissionRequest.is_queueable());
        assert!(HookEventName::Stop.is_queueable());
        assert!(!HookEventName::Notification.is_queueable());
        assert!(!HookEventName::PreToolUse.is_queueable());
        assert!(!HookEventName::PostToolUse.is_queueable());
    }
}
