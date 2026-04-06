use std::path::Path;

use anyhow::{Context, Result};

use crate::provider::{HookSetupContext, LaunchCommand, PromptDelivery, Provider};

/// Hook event names that Claude Code supports.
const MANAGED_EVENTS: &[&str] = &[
    "PermissionRequest",
    "PostToolUse",
    "PostToolUseFailure",
    "Stop",
    "Notification",
    "UserPromptSubmit",
    "SubagentStart",
];

/// Claude Code AI coding agent provider.
pub struct ClaudeCodeProvider;

impl Provider for ClaudeCodeProvider {
    fn name(&self) -> &str {
        "Claude Code"
    }

    fn id(&self) -> &str {
        "claude-code"
    }

    fn launch_command(&self, working_dir: &Path, _prompt: &str) -> Result<LaunchCommand> {
        let wd_escaped = shell_escape(working_dir.display().to_string());
        let command = format!("cd {} && claude", wd_escaped);
        Ok(LaunchCommand {
            command,
            prompt_delivery: PromptDelivery::TypeAfterDelay { delay_secs: 2 },
        })
    }

    fn install_hooks(&self, ctx: &HookSetupContext) -> Result<()> {
        write_hook_config(ctx)
    }

    fn cleanup_hooks(&self, ctx: &HookSetupContext) -> Result<()> {
        remove_hook_config(ctx.working_dir)
    }

    fn managed_hook_events(&self) -> Vec<String> {
        MANAGED_EVENTS.iter().map(|s| (*s).to_string()).collect()
    }
}

/// Escape a string for safe use in shell single quotes.
fn shell_escape(s: String) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Write Claude Code hook configuration for a session (blocking I/O).
///
/// Merges with any existing `.claude/settings.local.json` to preserve
/// user hooks and settings.
fn write_hook_config(ctx: &HookSetupContext) -> Result<()> {
    let claude_dir = ctx.working_dir.join(".claude");
    std::fs::create_dir_all(&claude_dir).context("creating .claude directory")?;

    let herald_cmd = format!(
        "CLAUDE_SESSION_ID={} HERALD_SOCKET={} python3 {}",
        ctx.session_id,
        ctx.socket_path.display(),
        ctx.hook_script_path.display()
    );

    let herald_hook_entry = serde_json::json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": herald_cmd
        }]
    });

    let config_path = claude_dir.join("settings.local.json");
    let mut config: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .context("reading existing settings.local.json")?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if config.get("hooks").is_none() {
        config["hooks"] = serde_json::json!({});
    }

    // Remove stale herald hooks
    if let Some(hooks) = config["hooks"].as_object_mut() {
        for event_name in MANAGED_EVENTS {
            if let Some(arr) = hooks.get_mut(*event_name).and_then(|v| v.as_array_mut()) {
                arr.retain(|entry| {
                    !entry["hooks"]
                        .as_array()
                        .is_some_and(|h| {
                            h.iter().any(|hook| {
                                hook["command"]
                                    .as_str()
                                    .is_some_and(|cmd| cmd.contains("herald-hook.py"))
                            })
                        })
                });
            }
        }
    }

    // Append herald hooks
    for event_name in MANAGED_EVENTS {
        let hooks_array = config["hooks"][*event_name]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut new_array = hooks_array;
        new_array.push(herald_hook_entry.clone());
        config["hooks"][*event_name] = serde_json::Value::Array(new_array);
    }

    std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)
        .context("writing hook config")?;

    Ok(())
}

/// Remove herald hook entries from `.claude/settings.local.json`.
fn remove_hook_config(working_dir: &Path) -> Result<()> {
    let config_path = working_dir.join(".claude/settings.local.json");
    if !config_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&config_path)
        .context("reading settings.local.json for cleanup")?;
    let mut config: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));

    if let Some(hooks) = config.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for event_name in MANAGED_EVENTS {
            if let Some(arr) = hooks.get_mut(*event_name).and_then(|v| v.as_array_mut()) {
                arr.retain(|entry| {
                    !entry["hooks"]
                        .as_array()
                        .is_some_and(|h| {
                            h.iter().any(|hook| {
                                hook["command"]
                                    .as_str()
                                    .is_some_and(|cmd| cmd.contains("herald-hook.py"))
                            })
                        })
                });
            }
        }
    }

    std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)
        .context("writing cleaned hook config")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::model::SessionId;

    fn test_context<'a>(
        runtime_dir: &'a Path,
        working_dir: &'a Path,
        session_id: &'a SessionId,
    ) -> HookSetupContext<'a> {
        HookSetupContext {
            session_id,
            runtime_dir,
            working_dir,
            socket_path: runtime_dir.join(format!("{}.sock", session_id)),
            hook_script_path: runtime_dir.join("herald-hook.py"),
        }
    }

    #[test]
    fn hook_config_generation() {
        let dir = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();
        let session_id = SessionId("test-session-123".into());
        let ctx = test_context(dir.path(), working_dir.path(), &session_id);

        write_hook_config(&ctx).unwrap();

        let config_path = working_dir.path().join(".claude/settings.local.json");
        assert!(config_path.exists());

        let content = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        let hooks = parsed.get("hooks").unwrap();
        assert!(hooks.get("PermissionRequest").is_some());
        assert!(hooks.get("PostToolUse").is_some());
        assert!(hooks.get("PostToolUseFailure").is_some());
        assert!(hooks.get("Stop").is_some());
        assert!(hooks.get("Notification").is_some());

        let perm_cmd = hooks["PermissionRequest"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(perm_cmd.contains("test-session-123"));
    }

    #[test]
    fn hook_config_merges_with_existing() {
        let dir = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();
        let claude_dir = working_dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        let existing = serde_json::json!({
            "hooks": {
                "Notification": [{
                    "matcher": "",
                    "hooks": [{
                        "type": "command",
                        "command": "afplay /System/Library/Sounds/Glass.aiff"
                    }]
                }]
            },
            "other_setting": true
        });
        std::fs::write(
            claude_dir.join("settings.local.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        let session_id = SessionId("session-456".into());
        let ctx = test_context(dir.path(), working_dir.path(), &session_id);
        write_hook_config(&ctx).unwrap();

        let content =
            std::fs::read_to_string(claude_dir.join("settings.local.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["other_setting"], serde_json::json!(true));

        let notif_hooks = parsed["hooks"]["Notification"].as_array().unwrap();
        assert_eq!(notif_hooks.len(), 2);
        assert!(notif_hooks[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("afplay"));
        assert!(notif_hooks[1]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("herald-hook.py"));
    }

    #[test]
    fn hook_config_cleans_stale_herald_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();

        let sid1 = SessionId("session-1".into());
        let ctx1 = test_context(dir.path(), working_dir.path(), &sid1);
        write_hook_config(&ctx1).unwrap();

        let sid2 = SessionId("session-2".into());
        let ctx2 = test_context(dir.path(), working_dir.path(), &sid2);
        write_hook_config(&ctx2).unwrap();

        let content = std::fs::read_to_string(
            working_dir.path().join(".claude/settings.local.json"),
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        let perm_hooks = parsed["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(perm_hooks.len(), 1);
        assert!(perm_hooks[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("session-2"));
    }

    #[test]
    fn cleanup_removes_herald_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();

        let sid = SessionId("session-cleanup".into());
        let ctx = test_context(dir.path(), working_dir.path(), &sid);
        write_hook_config(&ctx).unwrap();

        remove_hook_config(working_dir.path()).unwrap();

        let content = std::fs::read_to_string(
            working_dir.path().join(".claude/settings.local.json"),
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        let perm_hooks = parsed["hooks"]["PermissionRequest"].as_array().unwrap();
        assert!(perm_hooks.is_empty());
    }

    #[test]
    fn cleanup_preserves_user_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();
        let claude_dir = working_dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        // Write user hook first
        let existing = serde_json::json!({
            "hooks": {
                "Notification": [{
                    "matcher": "",
                    "hooks": [{
                        "type": "command",
                        "command": "afplay /System/Library/Sounds/Glass.aiff"
                    }]
                }]
            }
        });
        std::fs::write(
            claude_dir.join("settings.local.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        // Add herald hooks
        let sid = SessionId("session-x".into());
        let ctx = test_context(dir.path(), working_dir.path(), &sid);
        write_hook_config(&ctx).unwrap();

        // Clean up herald hooks
        remove_hook_config(working_dir.path()).unwrap();

        let content =
            std::fs::read_to_string(claude_dir.join("settings.local.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        // User hook preserved
        let notif_hooks = parsed["hooks"]["Notification"].as_array().unwrap();
        assert_eq!(notif_hooks.len(), 1);
        assert!(notif_hooks[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("afplay"));
    }

    #[test]
    fn launch_command_format() {
        let provider = ClaudeCodeProvider;
        let cmd = provider
            .launch_command(Path::new("/home/user/project"), "fix tests")
            .unwrap();
        assert!(cmd.command.contains("cd '/home/user/project' && claude"));
        assert!(!cmd.command.contains("--worktree"));
        assert!(matches!(
            cmd.prompt_delivery,
            PromptDelivery::TypeAfterDelay { delay_secs: 2 }
        ));
    }

    #[test]
    fn provider_metadata() {
        let provider = ClaudeCodeProvider;
        assert_eq!(provider.name(), "Claude Code");
        assert_eq!(provider.id(), "claude-code");
        assert!(provider.managed_hook_events().contains(&"PermissionRequest".to_string()));
        assert!(provider.managed_hook_events().contains(&"Stop".to_string()));
    }
}
