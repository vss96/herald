pub mod claude_code;
pub mod registry;

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::session::model::SessionId;

/// Context passed to a provider for hook setup/teardown.
pub struct HookSetupContext<'a> {
    pub session_id: &'a SessionId,
    pub runtime_dir: &'a Path,
    pub working_dir: &'a Path,
    pub socket_path: PathBuf,
    pub hook_script_path: PathBuf,
}

/// Result of generating a launch command for a session.
pub struct LaunchCommand {
    /// The shell command string to send to the tmux pane.
    pub command: String,
    /// How to deliver the initial prompt to the provider.
    pub prompt_delivery: PromptDelivery,
}

/// How the initial prompt is delivered to the provider after launch.
pub enum PromptDelivery {
    /// Type the prompt as keystrokes after a delay (Claude Code model).
    TypeAfterDelay { delay_secs: u64 },
    /// Prompt is already embedded in the launch command.
    InCommand,
}

/// Abstraction over AI coding agent providers (Claude Code, Codex, etc.).
///
/// Object-safe: no generics, no async. Call site wraps blocking methods
/// with `spawn_blocking` as needed.
pub trait Provider: Send + Sync {
    /// Human-readable name for UI display (e.g., "Claude Code").
    fn name(&self) -> &str;

    /// Short identifier for config/serialization (e.g., "claude-code").
    fn id(&self) -> &str;

    /// Generate the shell command to launch this provider in a tmux pane.
    fn launch_command(&self, working_dir: &Path, prompt: &str) -> Result<LaunchCommand>;

    /// Write/install hook configuration so the provider sends events
    /// to Herald's Unix socket. Blocking filesystem I/O.
    fn install_hooks(&self, ctx: &HookSetupContext) -> Result<()>;

    /// Clean up hook configuration when a session ends. Best-effort.
    fn cleanup_hooks(&self, ctx: &HookSetupContext) -> Result<()>;

    /// Hook event names this provider subscribes to (written into config).
    fn managed_hook_events(&self) -> Vec<String>;
}
