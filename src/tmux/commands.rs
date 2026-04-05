use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Result of a tmux command execution.
#[derive(Debug)]
pub struct TmuxOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Run a tmux command on the blocking thread pool (safe from async context).
async fn run_tmux(args: Vec<String>) -> Result<TmuxOutput> {
    tokio::task::spawn_blocking(move || {
        let output = Command::new("tmux")
            .args(&args)
            .output()
            .context("failed to execute tmux")?;

        Ok(TmuxOutput {
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            success: output.status.success(),
        })
    })
    .await?
}

/// Helper to convert &str args to owned Strings for spawn_blocking.
fn args(strs: &[&str]) -> Vec<String> {
    strs.iter().map(|s| s.to_string()).collect()
}

pub async fn has_session(name: &str) -> Result<bool> {
    let output = run_tmux(args(&["has-session", "-t", name])).await?;
    Ok(output.success)
}

pub async fn new_session(name: &str) -> Result<()> {
    let output = run_tmux(args(&["new-session", "-d", "-s", name])).await?;
    if !output.success {
        anyhow::bail!("failed to create session '{}': {}", name, output.stderr);
    }
    Ok(())
}

pub async fn new_window(session: &str, window_name: &str) -> Result<String> {
    let output = run_tmux(args(&[
        "new-window", "-t", session, "-n", window_name, "-P", "-F", "#{pane_id}",
    ]))
    .await?;
    if !output.success {
        anyhow::bail!(
            "failed to create window '{}' in '{}': {}",
            window_name,
            session,
            output.stderr
        );
    }
    Ok(output.stdout)
}

pub async fn send_keys(pane_id: &str, keys: &str) -> Result<()> {
    let output = run_tmux(args(&["send-keys", "-t", pane_id, keys])).await?;
    if !output.success {
        anyhow::bail!("failed to send keys to '{}': {}", pane_id, output.stderr);
    }
    Ok(())
}

pub async fn set_pane_option(pane_id: &str, key: &str, value: &str) -> Result<()> {
    let output = run_tmux(args(&["set-option", "-p", "-t", pane_id, key, value])).await?;
    if !output.success {
        anyhow::bail!(
            "failed to set option '{}' on '{}': {}",
            key,
            pane_id,
            output.stderr
        );
    }
    Ok(())
}

pub async fn list_panes(session: &str, format: &str) -> Result<Vec<String>> {
    let output = run_tmux(args(&["list-panes", "-s", "-t", session, "-F", format])).await?;
    if !output.success {
        anyhow::bail!("failed to list panes in '{}': {}", session, output.stderr);
    }
    Ok(output
        .stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}

pub async fn kill_window(session: &str, window_name: &str) -> Result<()> {
    let target = format!("{}:{}", session, window_name);
    let output = run_tmux(args(&["kill-window", "-t", &target])).await?;
    if !output.success {
        anyhow::bail!("failed to kill window '{}': {}", target, output.stderr);
    }
    Ok(())
}

pub async fn kill_session(name: &str) -> Result<()> {
    let output = run_tmux(args(&["kill-session", "-t", name])).await?;
    if !output.success {
        anyhow::bail!("failed to kill session '{}': {}", name, output.stderr);
    }
    Ok(())
}

/// Capture the visible content of a pane with ANSI escape sequences.
pub async fn capture_pane(pane_id: &str) -> Result<String> {
    let output = run_tmux(args(&["capture-pane", "-p", "-e", "-t", pane_id])).await?;
    Ok(output.stdout)
}

/// Kill a specific pane by ID.
pub async fn kill_pane(pane_id: &str) -> Result<()> {
    let output = run_tmux(args(&["kill-pane", "-t", pane_id])).await?;
    if !output.success {
        anyhow::bail!("failed to kill pane '{}': {}", pane_id, output.stderr);
    }
    Ok(())
}

/// Send literal text to a pane (handles special characters properly).
pub async fn send_keys_literal(pane_id: &str, keys: &str) -> Result<()> {
    let output = run_tmux(args(&["send-keys", "-l", "-t", pane_id, keys])).await?;
    if !output.success {
        anyhow::bail!("failed to send literal keys to '{}': {}", pane_id, output.stderr);
    }
    Ok(())
}

/// Send a special key (Enter, Escape, etc.) to a pane.
pub async fn send_special_key(pane_id: &str, key: &str) -> Result<()> {
    let output = run_tmux(args(&["send-keys", "-t", pane_id, key])).await?;
    if !output.success {
        anyhow::bail!("failed to send key '{}' to '{}': {}", key, pane_id, output.stderr);
    }
    Ok(())
}
