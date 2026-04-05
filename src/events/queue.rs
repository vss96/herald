use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::events::types::{HookEvent, HookEventName, Priority};

/// Reason a session needs attention.
#[derive(Debug, Clone, PartialEq)]
pub enum AttentionReason {
    PermissionPrompt {
        tool_name: String,
        tool_use_id: Option<String>,
    },
    ToolError {
        tool_name: String,
    },
    Completed,
}

/// An entry in the attention queue.
#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub session_id: String,
    pub priority: Priority,
    pub reason: AttentionReason,
    pub entered_at: Instant,
    pub tool_use_id: Option<String>,
}

/// Priority queue with fairness, debounce, and resolution-based clearing.
///
/// Invariants:
/// - At most one entry per session
/// - Higher priority events replace lower ones
/// - Entries persist until resolved (not cleared by focus)
/// - Same-session errors debounced within 2 seconds
/// - Re-entry cooldown of 5 seconds per priority tier
/// - Stale entries (>5 min) can be manually dismissed
pub struct AttentionQueue {
    entries: HashMap<String, QueueEntry>,
    /// Last time a session entered a given priority tier (for fairness cooldown)
    last_entry_time: HashMap<(String, Priority), Instant>,
    /// Debounce window for error coalescing
    debounce_duration: Duration,
    /// Cooldown before a session can re-enter the same tier
    fairness_cooldown: Duration,
    /// Entries older than this are considered stale
    stale_threshold: Duration,
}

impl AttentionQueue {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            last_entry_time: HashMap::new(),
            debounce_duration: Duration::from_secs(2),
            fairness_cooldown: Duration::from_secs(5),
            stale_threshold: Duration::from_secs(300), // 5 minutes
        }
    }

    /// For testing: create with custom durations.
    #[cfg(test)]
    fn with_config(debounce: Duration, cooldown: Duration, stale: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            last_entry_time: HashMap::new(),
            debounce_duration: debounce,
            fairness_cooldown: cooldown,
            stale_threshold: stale,
        }
    }

    /// Process a hook event. Returns true if the queue changed.
    pub fn process_event(&mut self, event: &HookEvent) -> bool {
        self.process_event_at(event, Instant::now())
    }

    /// Process a hook event at a specific time (for testing).
    fn process_event_at(&mut self, event: &HookEvent, now: Instant) -> bool {
        let priority = event.hook_event_name.priority();

        // PostToolUse resolves pending permission prompts
        if event.hook_event_name == HookEventName::PostToolUse {
            return self.try_resolve_permission(&event.session_id, event.tool_use_id.as_deref());
        }

        // Non-queueable events don't enter the queue
        if !event.hook_event_name.is_queueable() {
            // But non-error events from a session can clear an error entry
            if let Some(existing) = self.entries.get(&event.session_id) {
                if matches!(existing.reason, AttentionReason::ToolError { .. }) {
                    self.entries.remove(&event.session_id);
                    return true;
                }
            }
            return false;
        }

        // Check fairness cooldown
        let tier_key = (event.session_id.clone(), priority);
        if let Some(&last_time) = self.last_entry_time.get(&tier_key) {
            if now.duration_since(last_time) < self.fairness_cooldown {
                return false; // Cooldown not expired
            }
        }

        // Check debounce for errors
        if priority == Priority::Critical {
            if let Some(existing) = self.entries.get(&event.session_id) {
                if existing.priority == Priority::Critical
                    && now.duration_since(existing.entered_at) < self.debounce_duration
                {
                    // Coalesce: update the entry but don't count as "changed"
                    return false;
                }
            }
        }

        // Only replace if new priority >= existing
        if let Some(existing) = self.entries.get(&event.session_id) {
            if priority < existing.priority {
                return false;
            }
        }

        let reason = match event.hook_event_name {
            HookEventName::PermissionRequest => AttentionReason::PermissionPrompt {
                tool_name: event.tool_name.clone().unwrap_or_default(),
                tool_use_id: event.tool_use_id.clone(),
            },
            HookEventName::PostToolUseFailure => AttentionReason::ToolError {
                tool_name: event.tool_name.clone().unwrap_or_default(),
            },
            HookEventName::Stop => AttentionReason::Completed,
            _ => return false,
        };

        self.entries.insert(
            event.session_id.clone(),
            QueueEntry {
                session_id: event.session_id.clone(),
                priority,
                reason,
                entered_at: now,
                tool_use_id: event.tool_use_id.clone(),
            },
        );
        self.last_entry_time.insert(tier_key, now);
        true
    }

    /// Try to resolve a permission prompt by matching tool_use_id.
    fn try_resolve_permission(&mut self, session_id: &str, tool_use_id: Option<&str>) -> bool {
        if let Some(entry) = self.entries.get(session_id) {
            if let AttentionReason::PermissionPrompt {
                tool_use_id: ref pending_id,
                ..
            } = entry.reason
            {
                // Match by tool_use_id if both are present
                let should_clear = match (tool_use_id, pending_id.as_deref()) {
                    (Some(resolve_id), Some(pending)) => resolve_id == pending,
                    // Fallback: if either is missing, clear on any PostToolUse from same session
                    _ => true,
                };
                if should_clear {
                    self.entries.remove(session_id);
                    return true;
                }
            }
        }
        false
    }

    /// Dismiss an error entry for a session (user explicit action).
    pub fn dismiss_error(&mut self, session_id: &str) -> bool {
        if let Some(entry) = self.entries.get(session_id) {
            if matches!(entry.reason, AttentionReason::ToolError { .. }) {
                self.entries.remove(session_id);
                return true;
            }
        }
        false
    }

    /// Dismiss a completion entry for a session (user viewed and dismissed).
    pub fn dismiss_completion(&mut self, session_id: &str) -> bool {
        if let Some(entry) = self.entries.get(session_id) {
            if matches!(entry.reason, AttentionReason::Completed) {
                self.entries.remove(session_id);
                return true;
            }
        }
        false
    }

    /// Get the highest-priority entry, respecting FIFO within tiers.
    pub fn peek(&self) -> Option<&QueueEntry> {
        self.entries
            .values()
            .max_by(|a, b| a.priority.cmp(&b.priority).then(b.entered_at.cmp(&a.entered_at)))
    }

    /// Get all entries sorted by priority (desc) then entry time (asc).
    pub fn entries_sorted(&self) -> Vec<&QueueEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.entered_at.cmp(&b.entered_at)));
        entries
    }

    /// Check if a session has a stale entry.
    pub fn is_stale(&self, session_id: &str) -> bool {
        self.is_stale_at(session_id, Instant::now())
    }

    fn is_stale_at(&self, session_id: &str, now: Instant) -> bool {
        self.entries
            .get(session_id)
            .is_some_and(|e| now.duration_since(e.entered_at) > self.stale_threshold)
    }

    /// Force-dismiss a stale entry.
    pub fn dismiss_stale(&mut self, session_id: &str) -> bool {
        if self.is_stale(session_id) {
            self.entries.remove(session_id);
            return true;
        }
        false
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, session_id: &str) -> Option<&QueueEntry> {
        self.entries.get(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(session_id: &str, name: HookEventName, tool_use_id: Option<&str>) -> HookEvent {
        HookEvent {
            session_id: session_id.to_string(),
            hook_event_name: name,
            tool_name: Some("Edit".to_string()),
            tool_use_id: tool_use_id.map(|s| s.to_string()),
            tool_input: None,
            cwd: None,
        }
    }

    // ── Basic queue behavior ──

    #[test]
    fn permission_request_enters_queue() {
        let mut q = AttentionQueue::new();
        let event = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        assert!(q.process_event(&event));
        assert_eq!(q.len(), 1);
        assert_eq!(q.get("s1").unwrap().priority, Priority::High);
    }

    #[test]
    fn error_enters_queue() {
        let mut q = AttentionQueue::new();
        let event = make_event("s1", HookEventName::PostToolUseFailure, None);
        assert!(q.process_event(&event));
        assert_eq!(q.get("s1").unwrap().priority, Priority::Critical);
    }

    #[test]
    fn stop_enters_queue() {
        let mut q = AttentionQueue::new();
        let event = make_event("s1", HookEventName::Stop, None);
        assert!(q.process_event(&event));
        assert_eq!(q.get("s1").unwrap().priority, Priority::Low);
    }

    #[test]
    fn info_events_dont_enter_queue() {
        let mut q = AttentionQueue::new();
        let event = make_event("s1", HookEventName::Notification, None);
        assert!(!q.process_event(&event));
        assert!(q.is_empty());
    }

    #[test]
    fn one_entry_per_session() {
        let mut q = AttentionQueue::new();
        let e1 = make_event("s1", HookEventName::Stop, None);
        let e2 = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        q.process_event(&e1);
        q.process_event(&e2);
        assert_eq!(q.len(), 1);
        // Higher priority replaced lower
        assert_eq!(q.get("s1").unwrap().priority, Priority::High);
    }

    #[test]
    fn lower_priority_does_not_replace_higher() {
        let mut q = AttentionQueue::new();
        let e1 = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        q.process_event(&e1);
        let e2 = make_event("s1", HookEventName::Stop, None);
        assert!(!q.process_event(&e2));
        assert_eq!(q.get("s1").unwrap().priority, Priority::High);
    }

    // ── Resolution-based clearing ──

    #[test]
    fn post_tool_use_clears_permission_by_tool_use_id() {
        let mut q = AttentionQueue::new();
        let perm = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        q.process_event(&perm);
        assert_eq!(q.len(), 1);

        let resolve = make_event("s1", HookEventName::PostToolUse, Some("t1"));
        assert!(q.process_event(&resolve));
        assert!(q.is_empty());
    }

    #[test]
    fn post_tool_use_wrong_id_does_not_clear() {
        let mut q = AttentionQueue::new();
        let perm = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        q.process_event(&perm);

        let resolve = make_event("s1", HookEventName::PostToolUse, Some("t_other"));
        assert!(!q.process_event(&resolve));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn post_tool_use_fallback_clears_when_no_id() {
        let mut q = AttentionQueue::new();
        // Permission without tool_use_id
        let perm = make_event("s1", HookEventName::PermissionRequest, None);
        q.process_event(&perm);

        // PostToolUse without tool_use_id — fallback should clear
        let resolve = make_event("s1", HookEventName::PostToolUse, None);
        assert!(q.process_event(&resolve));
        assert!(q.is_empty());
    }

    #[test]
    fn non_error_info_event_clears_error_entry() {
        let mut q = AttentionQueue::new();
        let error = make_event("s1", HookEventName::PostToolUseFailure, None);
        q.process_event(&error);
        assert_eq!(q.len(), 1);

        // A Notification (info) from same session should clear the error
        let notif = make_event("s1", HookEventName::Notification, None);
        assert!(q.process_event(&notif));
        assert!(q.is_empty());
    }

    // ── Dismiss actions ──

    #[test]
    fn dismiss_error() {
        let mut q = AttentionQueue::new();
        let error = make_event("s1", HookEventName::PostToolUseFailure, None);
        q.process_event(&error);
        assert!(q.dismiss_error("s1"));
        assert!(q.is_empty());
    }

    #[test]
    fn dismiss_error_on_non_error_is_noop() {
        let mut q = AttentionQueue::new();
        let perm = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        q.process_event(&perm);
        assert!(!q.dismiss_error("s1"));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn dismiss_completion() {
        let mut q = AttentionQueue::new();
        let stop = make_event("s1", HookEventName::Stop, None);
        q.process_event(&stop);
        assert!(q.dismiss_completion("s1"));
        assert!(q.is_empty());
    }

    // ── Ordering ──

    #[test]
    fn peek_returns_highest_priority() {
        let mut q = AttentionQueue::new();
        let stop = make_event("s1", HookEventName::Stop, None);
        q.process_event(&stop);
        let error = make_event("s2", HookEventName::PostToolUseFailure, None);
        q.process_event(&error);
        let perm = make_event("s3", HookEventName::PermissionRequest, Some("t1"));
        q.process_event(&perm);

        assert_eq!(q.peek().unwrap().session_id, "s2"); // Critical > High > Low
    }

    #[test]
    fn fifo_within_same_priority() {
        let mut q = AttentionQueue::new();
        let now = Instant::now();

        let e1 = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        q.process_event_at(&e1, now);
        let e2 = make_event("s2", HookEventName::PermissionRequest, Some("t2"));
        q.process_event_at(&e2, now + Duration::from_millis(100));

        // s1 entered first, should be peeked first
        assert_eq!(q.peek().unwrap().session_id, "s1");
    }

    // ── Debounce ──

    #[test]
    fn error_debounce_within_window() {
        let mut q = AttentionQueue::with_config(
            Duration::from_secs(2),
            Duration::ZERO, // disable cooldown for this test
            Duration::from_secs(300),
        );
        let now = Instant::now();

        let e1 = make_event("s1", HookEventName::PostToolUseFailure, None);
        assert!(q.process_event_at(&e1, now));

        // Second error within 2s debounce window
        let e2 = make_event("s1", HookEventName::PostToolUseFailure, None);
        assert!(!q.process_event_at(&e2, now + Duration::from_secs(1)));
    }

    #[test]
    fn error_debounce_after_window() {
        let mut q = AttentionQueue::with_config(
            Duration::from_secs(2),
            Duration::ZERO,
            Duration::from_secs(300),
        );
        let now = Instant::now();

        let e1 = make_event("s1", HookEventName::PostToolUseFailure, None);
        q.process_event_at(&e1, now);

        // Error after debounce window
        let e2 = make_event("s1", HookEventName::PostToolUseFailure, None);
        assert!(q.process_event_at(&e2, now + Duration::from_secs(3)));
    }

    // ── Fairness cooldown ──

    #[test]
    fn fairness_cooldown_blocks_rapid_reentry() {
        let mut q = AttentionQueue::with_config(
            Duration::ZERO,
            Duration::from_secs(5),
            Duration::from_secs(300),
        );
        let now = Instant::now();

        let e1 = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        assert!(q.process_event_at(&e1, now));

        // Resolve the permission
        let resolve = make_event("s1", HookEventName::PostToolUse, Some("t1"));
        q.process_event_at(&resolve, now + Duration::from_secs(1));
        assert!(q.is_empty());

        // Try to re-enter same tier within cooldown
        let e2 = make_event("s1", HookEventName::PermissionRequest, Some("t2"));
        assert!(!q.process_event_at(&e2, now + Duration::from_secs(2)));
    }

    #[test]
    fn fairness_cooldown_allows_after_expiry() {
        let mut q = AttentionQueue::with_config(
            Duration::ZERO,
            Duration::from_secs(5),
            Duration::from_secs(300),
        );
        let now = Instant::now();

        let e1 = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        q.process_event_at(&e1, now);
        let resolve = make_event("s1", HookEventName::PostToolUse, Some("t1"));
        q.process_event_at(&resolve, now + Duration::from_secs(1));

        // Re-enter after cooldown expired
        let e2 = make_event("s1", HookEventName::PermissionRequest, Some("t2"));
        assert!(q.process_event_at(&e2, now + Duration::from_secs(6)));
    }

    // ── Stale entries ──

    #[test]
    fn stale_detection() {
        let mut q = AttentionQueue::with_config(
            Duration::ZERO,
            Duration::ZERO,
            Duration::from_secs(300),
        );
        let now = Instant::now();

        let e = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        q.process_event_at(&e, now);

        assert!(!q.is_stale_at("s1", now + Duration::from_secs(299)));
        assert!(q.is_stale_at("s1", now + Duration::from_secs(301)));
    }

    // ── Multiple sessions ──

    #[test]
    fn multiple_sessions_independent() {
        let mut q = AttentionQueue::new();
        let e1 = make_event("s1", HookEventName::PermissionRequest, Some("t1"));
        let e2 = make_event("s2", HookEventName::PostToolUseFailure, None);
        let e3 = make_event("s3", HookEventName::Stop, None);
        q.process_event(&e1);
        q.process_event(&e2);
        q.process_event(&e3);
        assert_eq!(q.len(), 3);

        // Resolving s1 doesn't affect s2 or s3
        let resolve = make_event("s1", HookEventName::PostToolUse, Some("t1"));
        q.process_event(&resolve);
        assert_eq!(q.len(), 2);
        assert!(q.get("s1").is_none());
        assert!(q.get("s2").is_some());
        assert!(q.get("s3").is_some());
    }
}
