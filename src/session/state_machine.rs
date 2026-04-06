use std::time::Instant;

use crate::events::types::HookEventName;
use crate::session::model::{AttentionReason, SessionStatus};

/// All events that can trigger a session state transition.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// A hook event from Claude Code.
    Hook {
        name: HookEventName,
        tool_name: Option<String>,
        tool_use_id: Option<String>,
    },
    /// User dismissed an attention entry.
    UserDismiss,
    /// User killed the session.
    UserKill,
}

/// Pure transition function: the single source of truth for session state changes.
///
/// Returns `Some(new_status)` if the state should change, `None` if no transition applies.
pub fn transition(current: &SessionStatus, event: &SessionEvent) -> Option<SessionStatus> {
    match event {
        SessionEvent::Hook { name, tool_name, tool_use_id } => {
            hook_transition(current, name, tool_name.as_deref(), tool_use_id.clone())
        }
        SessionEvent::UserDismiss => dismiss_transition(current),
        SessionEvent::UserKill => Some(SessionStatus::Stopped),
    }
}

/// Transitions triggered by hook events from Claude Code.
fn hook_transition(
    current: &SessionStatus,
    name: &HookEventName,
    tool_name: Option<&str>,
    tool_use_id: Option<String>,
) -> Option<SessionStatus> {
    // Terminal states reject all events
    match current {
        SessionStatus::Stopped | SessionStatus::Error { .. } => return None,
        _ => {}
    }

    match name {
        HookEventName::PermissionRequest => Some(SessionStatus::NeedsAttention {
            reason: AttentionReason::PermissionPrompt {
                tool_name: tool_name.unwrap_or("").to_string(),
                tool_use_id,
            },
            since: Instant::now(),
        }),
        HookEventName::PostToolUseFailure => Some(SessionStatus::NeedsAttention {
            reason: AttentionReason::ToolError {
                tool_name: tool_name.unwrap_or("").to_string(),
                error: String::new(),
            },
            since: Instant::now(),
        }),
        HookEventName::Stop => Some(SessionStatus::NeedsAttention {
            reason: AttentionReason::Completed,
            since: Instant::now(),
        }),
        HookEventName::PostToolUse
        | HookEventName::PreToolUse
        | HookEventName::Notification
        | HookEventName::UserPromptSubmit
        | HookEventName::SubagentStart => Some(SessionStatus::Running {
            last_activity: Instant::now(),
        }),
        HookEventName::SessionStart | HookEventName::SessionEnd => None,
    }
}

/// Transitions triggered by user dismissing an attention entry.
fn dismiss_transition(current: &SessionStatus) -> Option<SessionStatus> {
    match current {
        SessionStatus::NeedsAttention { reason: AttentionReason::Completed, .. } => {
            Some(SessionStatus::Stopped)
        }
        SessionStatus::NeedsAttention { reason: AttentionReason::ToolError { .. }, .. } => {
            Some(SessionStatus::Error {
                message: "dismissed".to_string(),
            })
        }
        SessionStatus::NeedsAttention { reason: AttentionReason::PermissionPrompt { .. }, .. } => {
            // Dismissing a permission prompt means user is acknowledging it;
            // session continues running
            Some(SessionStatus::Running {
                last_activity: Instant::now(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> SessionStatus {
        SessionStatus::Running {
            last_activity: Instant::now(),
        }
    }

    fn needs_attention_permission() -> SessionStatus {
        SessionStatus::NeedsAttention {
            reason: AttentionReason::PermissionPrompt {
                tool_name: "Edit".into(),
                tool_use_id: None,
            },
            since: Instant::now(),
        }
    }

    fn needs_attention_error() -> SessionStatus {
        SessionStatus::NeedsAttention {
            reason: AttentionReason::ToolError {
                tool_name: "Bash".into(),
                error: "fail".into(),
            },
            since: Instant::now(),
        }
    }

    fn needs_attention_completed() -> SessionStatus {
        SessionStatus::NeedsAttention {
            reason: AttentionReason::Completed,
            since: Instant::now(),
        }
    }

    fn hook(name: HookEventName) -> SessionEvent {
        SessionEvent::Hook { name, tool_name: None, tool_use_id: None }
    }

    fn hook_with_tool(name: HookEventName, tool: &str) -> SessionEvent {
        SessionEvent::Hook {
            name,
            tool_name: Some(tool.to_string()),
            tool_use_id: None,
        }
    }

    // ── Hook transitions ──

    #[test]
    fn starting_to_running_on_activity() {
        for event in [
            HookEventName::PostToolUse,
            HookEventName::PreToolUse,
            HookEventName::Notification,
            HookEventName::UserPromptSubmit,
            HookEventName::SubagentStart,
        ] {
            let result = transition(&SessionStatus::Starting, &hook(event.clone()));
            assert!(
                matches!(result, Some(SessionStatus::Running { .. })),
                "expected Running for {:?}",
                event
            );
        }
    }

    #[test]
    fn starting_to_needs_attention_on_permission() {
        let result = transition(
            &SessionStatus::Starting,
            &hook_with_tool(HookEventName::PermissionRequest, "Edit"),
        );
        assert!(matches!(
            result,
            Some(SessionStatus::NeedsAttention {
                reason: AttentionReason::PermissionPrompt { .. },
                ..
            })
        ));
    }

    #[test]
    fn starting_to_needs_attention_on_error() {
        let result = transition(
            &SessionStatus::Starting,
            &hook(HookEventName::PostToolUseFailure),
        );
        assert!(matches!(
            result,
            Some(SessionStatus::NeedsAttention {
                reason: AttentionReason::ToolError { .. },
                ..
            })
        ));
    }

    #[test]
    fn starting_to_needs_attention_on_stop() {
        let result = transition(&SessionStatus::Starting, &hook(HookEventName::Stop));
        assert!(matches!(
            result,
            Some(SessionStatus::NeedsAttention {
                reason: AttentionReason::Completed,
                ..
            })
        ));
    }

    #[test]
    fn running_to_needs_attention() {
        let result = transition(&running(), &hook(HookEventName::PermissionRequest));
        assert!(matches!(
            result,
            Some(SessionStatus::NeedsAttention {
                reason: AttentionReason::PermissionPrompt { .. },
                ..
            })
        ));
    }

    #[test]
    fn needs_attention_to_running_on_activity() {
        let result = transition(
            &needs_attention_permission(),
            &hook(HookEventName::PreToolUse),
        );
        assert!(matches!(result, Some(SessionStatus::Running { .. })));
    }

    #[test]
    fn needs_attention_can_change_reason() {
        // ToolError can be replaced by PermissionRequest
        let result = transition(
            &needs_attention_error(),
            &hook(HookEventName::PermissionRequest),
        );
        assert!(matches!(
            result,
            Some(SessionStatus::NeedsAttention {
                reason: AttentionReason::PermissionPrompt { .. },
                ..
            })
        ));
    }

    // ── Terminal states ──

    #[test]
    fn stopped_rejects_all_hook_events() {
        for event in [
            HookEventName::PostToolUse,
            HookEventName::PermissionRequest,
            HookEventName::Stop,
            HookEventName::Notification,
        ] {
            let result = transition(&SessionStatus::Stopped, &hook(event.clone()));
            assert!(result.is_none(), "Stopped should reject {:?}", event);
        }
    }

    #[test]
    fn error_rejects_all_hook_events() {
        let state = SessionStatus::Error { message: "crash".into() };
        for event in [
            HookEventName::PostToolUse,
            HookEventName::PermissionRequest,
            HookEventName::Stop,
        ] {
            let result = transition(&state, &hook(event.clone()));
            assert!(result.is_none(), "Error should reject {:?}", event);
        }
    }

    // ── UserDismiss transitions ──

    #[test]
    fn dismiss_completed_to_stopped() {
        let result = transition(&needs_attention_completed(), &SessionEvent::UserDismiss);
        assert!(matches!(result, Some(SessionStatus::Stopped)));
    }

    #[test]
    fn dismiss_error_to_error() {
        let result = transition(&needs_attention_error(), &SessionEvent::UserDismiss);
        assert!(matches!(result, Some(SessionStatus::Error { .. })));
    }

    #[test]
    fn dismiss_permission_to_running() {
        let result = transition(&needs_attention_permission(), &SessionEvent::UserDismiss);
        assert!(matches!(result, Some(SessionStatus::Running { .. })));
    }

    #[test]
    fn dismiss_running_is_noop() {
        let result = transition(&running(), &SessionEvent::UserDismiss);
        assert!(result.is_none());
    }

    // ── UserKill transitions ──

    #[test]
    fn kill_always_stops() {
        for state in [
            SessionStatus::Starting,
            running(),
            needs_attention_permission(),
            needs_attention_completed(),
            SessionStatus::Stopped,
            SessionStatus::Error { message: "x".into() },
        ] {
            let result = transition(&state, &SessionEvent::UserKill);
            assert!(
                matches!(result, Some(SessionStatus::Stopped)),
                "UserKill should produce Stopped from {:?}",
                state
            );
        }
    }

    // ── SessionStart/SessionEnd are no-ops ──

    #[test]
    fn session_lifecycle_events_are_noop() {
        for event in [HookEventName::SessionStart, HookEventName::SessionEnd] {
            let result = transition(&running(), &hook(event.clone()));
            assert!(result.is_none(), "{:?} should be a no-op", event);
        }
    }

    // ── Full lifecycle ──

    #[test]
    fn full_lifecycle_starting_to_stopped() {
        let mut status = SessionStatus::Starting;

        // Activity → Running
        status = transition(&status, &hook(HookEventName::PostToolUse)).unwrap();
        assert!(matches!(status, SessionStatus::Running { .. }));

        // Stop → NeedsAttention(Completed)
        status = transition(&status, &hook(HookEventName::Stop)).unwrap();
        assert!(matches!(
            status,
            SessionStatus::NeedsAttention { reason: AttentionReason::Completed, .. }
        ));

        // Dismiss → Stopped
        status = transition(&status, &SessionEvent::UserDismiss).unwrap();
        assert!(matches!(status, SessionStatus::Stopped));

        // Terminal — no more transitions
        assert!(transition(&status, &hook(HookEventName::PostToolUse)).is_none());
    }

    #[test]
    fn permission_request_captures_tool_name() {
        let result = transition(
            &running(),
            &hook_with_tool(HookEventName::PermissionRequest, "Bash"),
        );
        if let Some(SessionStatus::NeedsAttention {
            reason: AttentionReason::PermissionPrompt { tool_name, .. },
            ..
        }) = result
        {
            assert_eq!(tool_name, "Bash");
        } else {
            panic!("expected PermissionPrompt with tool_name");
        }
    }
}
