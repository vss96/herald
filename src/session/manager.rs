use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::session::model::Session;
use crate::tmux::commands;

const TMUX_SESSION_NAME: &str = "herald";
const PANE_METADATA_KEY: &str = "@herald_session_id";

/// Manages the lifecycle of all Claude Code sessions.
pub struct SessionManager {
    sessions: HashMap<String, Session>,
    runtime_dir: PathBuf,
    terminal_cols: u16,
    terminal_rows: u16,
}

impl SessionManager {
    pub fn new(runtime_dir: PathBuf, terminal_cols: u16, terminal_rows: u16) -> Self {
        Self {
            sessions: HashMap::new(),
            runtime_dir,
            terminal_cols,
            terminal_rows,
        }
    }

    /// Ensure the tmux session exists.
    pub async fn ensure_tmux_session(&self) -> Result<()> {
        if !commands::has_session(TMUX_SESSION_NAME).await? {
            commands::new_session(TMUX_SESSION_NAME).await?;
        }
        Ok(())
    }

    /// Launch a new Claude Code session.
    pub async fn launch(
        &mut self,
        nickname: &str,
        prompt: &str,
        working_dir: &Path,
    ) -> Result<String> {
        let session_id = uuid::Uuid::new_v4().to_string();

        let pane_id = commands::new_window(TMUX_SESSION_NAME, nickname).await?;
        commands::set_pane_option(&pane_id, PANE_METADATA_KEY, &session_id).await?;

        // write_hook_config uses std::fs — run on blocking pool
        let rt_dir = self.runtime_dir.clone();
        let wd = working_dir.to_path_buf();
        let sid = session_id.clone();
        tokio::task::spawn_blocking(move || {
            write_hook_config(&rt_dir, &wd, &sid)
        })
        .await??;

        // Write prompt to a temp file to avoid shell injection.
        let prompt_file = self.runtime_dir.join(format!("{}.prompt", session_id));
        tokio::fs::write(&prompt_file, prompt)
            .await
            .context("writing prompt file")?;

        let wd_escaped = shell_escape(working_dir.display().to_string());
        // Launch claude interactively (stays alive after completing work).
        // Fall back to plain claude if repo has no commits (worktree needs HEAD).
        let cmd = format!(
            "cd {} && if git rev-parse HEAD >/dev/null 2>&1; then claude --worktree; else claude; fi",
            wd_escaped,
        );
        commands::send_keys(&pane_id, &cmd).await?;
        commands::send_keys(&pane_id, "Enter").await?;

        // Wait for claude to start, then type the prompt as first message
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        commands::send_keys_literal(&pane_id, prompt).await?;
        commands::send_special_key(&pane_id, "Enter").await?;

        // Clean up prompt file (no longer needed)
        let _ = tokio::fs::remove_file(&prompt_file).await;

        let mut session = Session::new(
            session_id.clone(),
            nickname.to_string(),
            prompt.to_string(),
            working_dir.to_path_buf(),
            self.terminal_cols,
            self.terminal_rows,
        );
        session.tmux_pane_id = pane_id;

        self.sessions.insert(session_id.clone(), session);
        Ok(session_id)
    }

    /// Kill a session by ID.
    pub async fn kill(&mut self, session_id: &str) -> Result<()> {
        if let Some(session) = self.sessions.get(session_id) {
            // Kill by pane ID (not nickname — nicknames aren't unique)
            let _ = commands::kill_pane(&session.tmux_pane_id).await;
            let rt_dir = self.runtime_dir.clone();
            let sid = session_id.to_string();
            let _ = tokio::fs::remove_file(rt_dir.join(format!("{}.sock", &sid))).await;
            let _ = tokio::fs::remove_file(rt_dir.join(format!("{}.buffer", &sid))).await;
            let _ = tokio::fs::remove_file(rt_dir.join(format!("{}.prompt", &sid))).await;
            let _ = tokio::fs::remove_file(rt_dir.join(format!("{}.lock", &sid))).await;
        }
        self.sessions.remove(session_id);
        Ok(())
    }

    pub fn rename(&mut self, session_id: &str, new_name: &str) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.nickname = new_name.to_string();
        }
    }

    pub fn get(&self, session_id: &str) -> Option<&Session> {
        self.sessions.get(session_id)
    }

    pub fn get_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(session_id)
    }

    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Insert a session directly (for testing only).
    #[cfg(test)]
    pub fn insert_test_session(&mut self, session: Session) {
        self.sessions.insert(session.id.clone(), session);
    }

    /// Discover existing sessions from a previous TUI instance.
    pub async fn discover_existing(&mut self) -> Result<Vec<String>> {
        if !commands::has_session(TMUX_SESSION_NAME).await? {
            return Ok(vec![]);
        }

        // Use tab delimiter — safe even if nickname contains spaces.
        // Include pane_current_command to filter out herald's own panes.
        let format = "#{pane_id}\t#{window_name}\t#{@herald_session_id}\t#{pane_current_command}";
        let panes = commands::list_panes(TMUX_SESSION_NAME, format).await?;

        let mut discovered = Vec::new();
        for line in panes {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() >= 4 && !parts[2].is_empty() {
                let pane_id = parts[0].to_string();
                let nickname = parts[1].to_string();
                let session_id = parts[2].to_string();
                let command = parts[3];

                // Skip panes running herald itself (the default shell from new-session)
                if command == "herald" {
                    tracing::info!(pane_id = %pane_id, "skipping herald's own pane");
                    continue;
                }

                let mut session = Session::new(
                    session_id.clone(),
                    nickname,
                    String::new(),
                    PathBuf::new(),
                    self.terminal_cols,
                    self.terminal_rows,
                );
                session.tmux_pane_id = pane_id;
                self.sessions.insert(session_id.clone(), session);
                discovered.push(session_id);
            }
        }

        Ok(discovered)
    }
}

/// Escape a string for safe use in shell single quotes.
fn shell_escape(s: String) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Capture the visible content of a session's tmux pane.
pub async fn capture_session_pane(pane_id: &str) -> Result<String> {
    commands::capture_pane(pane_id).await
}

/// Write Claude Code hook configuration for a session (blocking I/O).
///
/// Merges with any existing `.claude/settings.local.json` to preserve
/// user hooks and settings.
fn write_hook_config(runtime_dir: &Path, working_dir: &Path, session_id: &str) -> Result<()> {
    let claude_dir = working_dir.join(".claude");
    std::fs::create_dir_all(&claude_dir).context("creating .claude directory")?;

    let hook_script = runtime_dir.join("herald-hook.py");
    let socket_path = runtime_dir.join(format!("{}.sock", session_id));

    let herald_cmd = format!(
        "CLAUDE_SESSION_ID={} HERALD_SOCKET={} python3 {}",
        session_id,
        socket_path.display(),
        hook_script.display()
    );

    let herald_hook_entry = serde_json::json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": herald_cmd
        }]
    });

    let hook_events = [
        "PermissionRequest",
        "PostToolUse",
        "PostToolUseFailure",
        "Stop",
        "Notification",
    ];

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
        for event_name in &hook_events {
            if let Some(arr) = hooks.get_mut(*event_name).and_then(|v| v.as_array_mut()) {
                arr.retain(|entry| {
                    !entry["hooks"]
                        .as_array()
                        .map_or(false, |h| {
                            h.iter().any(|hook| {
                                hook["command"]
                                    .as_str()
                                    .map_or(false, |cmd| cmd.contains("herald-hook.py"))
                            })
                        })
                });
            }
        }
    }

    // Append herald hooks
    for event_name in &hook_events {
        let hooks_array = config["hooks"][event_name]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut new_array = hooks_array;
        new_array.push(herald_hook_entry.clone());
        config["hooks"][event_name] = serde_json::Value::Array(new_array);
    }

    std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)
        .context("writing hook config")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_config_generation() {
        let dir = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();
        write_hook_config(dir.path(), working_dir.path(), "test-session-123").unwrap();

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

        write_hook_config(dir.path(), working_dir.path(), "session-456").unwrap();

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

        write_hook_config(dir.path(), working_dir.path(), "session-1").unwrap();
        write_hook_config(dir.path(), working_dir.path(), "session-2").unwrap();

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
}
