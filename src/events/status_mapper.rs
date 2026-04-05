use std::time::Instant;

use crate::events::types::HookEventName;
use crate::session::model::{AttentionReason, SessionStatus};

/// Pure function that maps a hook event to the next session status.
///
/// Returns `Some(new_status)` if the status should change, `None` if no change is needed.
pub fn next_status(
    current: &SessionStatus,
    event_name: &HookEventName,
    tool_name: Option<&str>,
) -> Option<SessionStatus> {
    match event_name {
        HookEventName::PermissionRequest => Some(SessionStatus::NeedsAttention {
            reason: AttentionReason::PermissionPrompt {
                tool_name: tool_name.unwrap_or("").to_string(),
                tool_use_id: None,
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
        HookEventName::PostToolUse | HookEventName::PreToolUse | HookEventName::Notification => {
            // Transition any non-terminal status to Running; don't resurrect Stopped/Error
            match current {
                SessionStatus::Starting
                | SessionStatus::Running { .. }
                | SessionStatus::NeedsAttention { .. } => Some(SessionStatus::Running {
                    last_activity: Instant::now(),
                }),
                SessionStatus::Stopped { .. } | SessionStatus::Error { .. } => None,
            }
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

    fn stopped() -> SessionStatus {
        SessionStatus::Stopped { exit_code: Some(0) }
    }

    fn error() -> SessionStatus {
        SessionStatus::Error {
            message: "crash".into(),
        }
    }

    // --- PermissionRequest ---

    #[test]
    fn permission_request_always_needs_attention() {
        let statuses = [
            SessionStatus::Starting,
            running(),
            needs_attention_permission(),
            stopped(),
            error(),
        ];
        for status in &statuses {
            let result = next_status(status, &HookEventName::PermissionRequest, Some("Edit"));
            assert!(
                matches!(
                    result,
                    Some(SessionStatus::NeedsAttention {
                        reason: AttentionReason::PermissionPrompt { .. },
                        ..
                    })
                ),
                "PermissionRequest should always yield NeedsAttention(PermissionPrompt), got {:?} for current={:?}",
                result, status
            );
        }
    }

    #[test]
    fn permission_request_captures_tool_name() {
        let result = next_status(&running(), &HookEventName::PermissionRequest, Some("Bash"));
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

    #[test]
    fn permission_request_no_tool_name_defaults_empty() {
        let result = next_status(&running(), &HookEventName::PermissionRequest, None);
        if let Some(SessionStatus::NeedsAttention {
            reason: AttentionReason::PermissionPrompt { tool_name, .. },
            ..
        }) = result
        {
            assert_eq!(tool_name, "");
        } else {
            panic!("expected PermissionPrompt");
        }
    }

    // --- PostToolUseFailure ---

    #[test]
    fn post_tool_use_failure_yields_tool_error() {
        let result = next_status(&running(), &HookEventName::PostToolUseFailure, Some("Bash"));
        assert!(
            matches!(
                result,
                Some(SessionStatus::NeedsAttention {
                    reason: AttentionReason::ToolError { .. },
                    ..
                })
            ),
            "expected NeedsAttention(ToolError)"
        );
    }

    // --- Stop ---

    #[test]
    fn stop_yields_completed() {
        let result = next_status(&running(), &HookEventName::Stop, None);
        assert!(
            matches!(
                result,
                Some(SessionStatus::NeedsAttention {
                    reason: AttentionReason::Completed,
                    ..
                })
            ),
            "expected NeedsAttention(Completed)"
        );
    }

    // --- PostToolUse / PreToolUse / Notification transitions ---

    #[test]
    fn active_events_transition_starting_to_running() {
        for event in [
            HookEventName::PostToolUse,
            HookEventName::PreToolUse,
            HookEventName::Notification,
        ] {
            let result = next_status(&SessionStatus::Starting, &event, None);
            assert!(
                matches!(result, Some(SessionStatus::Running { .. })),
                "expected Running for {:?} from Starting",
                event
            );
        }
    }

    #[test]
    fn active_events_transition_running_to_running() {
        let result = next_status(&running(), &HookEventName::PostToolUse, None);
        assert!(matches!(result, Some(SessionStatus::Running { .. })));
    }

    #[test]
    fn active_events_transition_needs_attention_to_running() {
        let result = next_status(&needs_attention_permission(), &HookEventName::PreToolUse, None);
        assert!(matches!(result, Some(SessionStatus::Running { .. })));
    }

    #[test]
    fn active_events_do_not_resurrect_stopped() {
        for event in [
            HookEventName::PostToolUse,
            HookEventName::PreToolUse,
            HookEventName::Notification,
        ] {
            let result = next_status(&stopped(), &event, None);
            assert!(
                result.is_none(),
                "should not resurrect Stopped for {:?}",
                event
            );
        }
    }

    #[test]
    fn active_events_do_not_resurrect_error() {
        let result = next_status(&error(), &HookEventName::Notification, None);
        assert!(result.is_none(), "should not resurrect Error");
    }

    // --- Other events (SessionStart, SessionEnd) ---

    #[test]
    fn other_events_return_none() {
        for event in [HookEventName::SessionStart, HookEventName::SessionEnd] {
            let result = next_status(&running(), &event, None);
            assert!(result.is_none(), "expected None for {:?}", event);
        }
    }
}
