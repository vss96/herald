use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::provider::registry::ProviderRegistry;
use crate::provider::{HookSetupContext, PromptDelivery};
use crate::session::model::{PaneId, Session, SessionId};
use crate::tmux::commands;

const TMUX_SESSION_NAME: &str = "herald";
const PANE_METADATA_KEY: &str = "@herald_session_id";
const PANE_PROVIDER_KEY: &str = "@herald_provider_id";
const PANE_WORKDIR_KEY: &str = "@herald_working_dir";
const PANE_WORKTREE_KEY: &str = "@herald_worktree_path";
const SESSION_FILE_EXTENSIONS: &[&str] = &["sock", "buffer", "prompt", "lock"];

/// Manages the lifecycle of AI coding sessions.
pub struct SessionManager {
    sessions: HashMap<SessionId, Session>,
    runtime_dir: PathBuf,
    terminal_rows: u16,
    registry: Arc<ProviderRegistry>,
}

impl SessionManager {
    pub fn new(runtime_dir: PathBuf, terminal_rows: u16, registry: Arc<ProviderRegistry>) -> Self {
        Self {
            sessions: HashMap::new(),
            runtime_dir,
            terminal_rows,
            registry,
        }
    }

    /// Ensure the tmux session exists.
    pub async fn ensure_tmux_session(&self) -> Result<()> {
        if !commands::has_session(TMUX_SESSION_NAME).await? {
            commands::new_session(TMUX_SESSION_NAME).await?;
        }
        Ok(())
    }

    /// Launch a new session with the specified provider.
    pub async fn launch(
        &mut self,
        nickname: &str,
        prompt: &str,
        working_dir: &Path,
        provider_id: &str,
        use_worktree: bool,
    ) -> Result<SessionId> {
        let provider = self.registry.get_by_id(provider_id)
            .ok_or_else(|| anyhow::anyhow!("unknown provider: {}", provider_id))?;

        let session_id = SessionId(uuid::Uuid::new_v4().to_string());

        // Create worktree if requested (before hook install so hooks target the worktree dir).
        // Also resolve the canonical repo root so kill() can later call
        // WorktreeManager::remove with an explicit repo path — the central
        // <data>/herald/worktrees/ layout means the worktree's parent chain
        // no longer identifies the source repo.
        let (worktree_path, repo_path) = if use_worktree {
            match crate::worktree::git_toplevel(working_dir).await {
                Ok(root) => match crate::worktree::WorktreeManager::create(
                    &root, nickname, &session_id,
                )
                .await
                {
                    Ok(path) => {
                        tracing::info!(worktree = %path.display(), "created worktree for session");
                        (Some(path), Some(root))
                    }
                    Err(e) => {
                        tracing::warn!("failed to create worktree, proceeding without: {}", e);
                        (None, None)
                    }
                },
                Err(e) => {
                    tracing::warn!("working dir is not in a git repo, skipping worktree: {}", e);
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        // Use worktree path as the effective working directory if available
        let effective_dir = worktree_path.as_deref().unwrap_or(working_dir);

        // Install hooks BEFORE creating the tmux pane. Running in the opposite
        // order meant a hook-install failure (bad permissions, malformed
        // settings.local.json, etc.) left the tmux pane orphaned with no
        // session record to clean it up on kill(). Install first; if that
        // fails, roll back the worktree and return — no pane to leak.
        let rt_dir = self.runtime_dir.clone();
        let wd = effective_dir.to_path_buf();
        let sid_clone = session_id.clone();
        let provider_id_owned = provider_id.to_string();
        let registry = self.registry.clone();
        let install_result = tokio::task::spawn_blocking(move || {
            let provider = registry
                .get_by_id(&provider_id_owned)
                .expect("provider disappeared from registry");
            let ctx = HookSetupContext {
                session_id: &sid_clone,
                working_dir: &wd,
                socket_path: rt_dir.join(format!("{}.sock", sid_clone)),
                hook_script_path: rt_dir.join("herald-hook.py"),
            };
            provider.install_hooks(&ctx)
        })
        .await?;

        if let Err(e) = install_result {
            rollback_worktree(&worktree_path, &repo_path).await;
            return Err(e);
        }

        // Create pane. If this fails, roll back hooks + worktree so a bad
        // tmux state doesn't leave stale Herald entries in settings.local.json.
        let pane_id = match commands::new_window(TMUX_SESSION_NAME, nickname).await {
            Ok(pid) => PaneId(pid),
            Err(e) => {
                rollback_hooks(
                    &self.registry,
                    provider_id,
                    &session_id,
                    effective_dir,
                    &self.runtime_dir,
                )
                .await;
                rollback_worktree(&worktree_path, &repo_path).await;
                return Err(e);
            }
        };
        // Persist all state discover_existing needs to faithfully reconstruct
        // the Session on restart: session id (used to match a pane to Herald),
        // provider id (so kill() routes cleanup through the right provider),
        // working dir (so hook cleanup targets the right .claude dir), and
        // worktree path (so kill() can remove the worktree git created).
        // Without these, a discovered session lost its provider (hardcoded to
        // "claude-code") and working dir (empty) so cleanup targeted the
        // wrong thing.
        let pane_options: &[(&str, &str)] = &[
            (PANE_METADATA_KEY, session_id.as_str()),
            (PANE_PROVIDER_KEY, provider_id),
            (
                PANE_WORKDIR_KEY,
                working_dir.to_str().unwrap_or(""),
            ),
            (
                PANE_WORKTREE_KEY,
                worktree_path
                    .as_deref()
                    .and_then(Path::to_str)
                    .unwrap_or(""),
            ),
        ];
        for (key, value) in pane_options {
            if let Err(e) = commands::set_pane_option(pane_id.as_str(), key, value).await {
                let _ = commands::kill_pane(pane_id.as_str()).await;
                rollback_hooks(
                    &self.registry,
                    provider_id,
                    &session_id,
                    effective_dir,
                    &self.runtime_dir,
                )
                .await;
                rollback_worktree(&worktree_path, &repo_path).await;
                return Err(e);
            }
        }

        // Generate and send launch command
        let launch_cmd = provider.launch_command(effective_dir, prompt)?;
        commands::send_keys(pane_id.as_str(), &launch_cmd.command).await?;
        commands::send_keys(pane_id.as_str(), "Enter").await?;

        // Handle prompt delivery based on provider's preference
        let PromptDelivery::TypeAfterDelay { delay_secs } = launch_cmd.prompt_delivery;
        // Write prompt to a temp file to avoid shell injection.
        let prompt_file = self.runtime_dir.join(format!("{}.prompt", session_id));
        tokio::fs::write(&prompt_file, prompt)
            .await
            .context("writing prompt file")?;

        tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
        commands::send_keys_literal(pane_id.as_str(), prompt).await?;
        commands::send_special_key(pane_id.as_str(), "Enter").await?;

        let _ = tokio::fs::remove_file(&prompt_file).await;

        let mut session = Session::new(
            session_id.clone(),
            nickname.to_string(),
            prompt.to_string(),
            working_dir.to_path_buf(),
            provider_id.to_string(),
        );
        session.tmux_pane_id = pane_id;
        session.worktree_path = worktree_path;
        session.repo_path = repo_path;

        self.sessions.insert(session_id.clone(), session);
        Ok(session_id)
    }

    /// Kill a session by ID.
    pub async fn kill(&mut self, session_id: &SessionId) -> Result<()> {
        if let Some(session) = self.sessions.get(session_id) {
            // Clean up provider hooks (best-effort). Use the same directory hooks
            // were installed into: the worktree path if the session has one, else
            // the original working dir. Using session.working_dir here would leave
            // worktree hooks behind.
            let rt_dir = self.runtime_dir.clone();
            let wd = session
                .worktree_path
                .as_deref()
                .unwrap_or(&session.working_dir)
                .to_path_buf();
            let sid = session_id.clone();
            let provider_id = session.provider_id.clone();
            let registry = self.registry.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Some(provider) = registry.get_by_id(&provider_id) {
                    let ctx = HookSetupContext {
                        session_id: &sid,
                        working_dir: &wd,
                        socket_path: rt_dir.join(format!("{}.sock", sid)),
                        hook_script_path: rt_dir.join("herald-hook.py"),
                    };
                    let _ = provider.cleanup_hooks(&ctx);
                }
            })
            .await;

            // Kill by pane ID (not nickname — nicknames aren't unique)
            let _ = commands::kill_pane(session.tmux_pane_id.as_str()).await;
            let rt_dir = self.runtime_dir.clone();
            let sid = session_id.to_string();
            for ext in SESSION_FILE_EXTENSIONS {
                let _ = tokio::fs::remove_file(rt_dir.join(format!("{}.{}", &sid, ext))).await;
            }

            // Clean up worktree if session had one. Awaited (not detached) so
            // that a user quitting Herald right after kill() doesn't cancel
            // the removal task and leave the worktree behind.
            if let (Some(wt_path), Some(repo)) = (&session.worktree_path, &session.repo_path) {
                if let Err(e) = crate::worktree::WorktreeManager::remove(repo, wt_path).await {
                    tracing::warn!("failed to clean up worktree: {}", e);
                }
            }
        }
        self.sessions.remove(session_id);
        Ok(())
    }

    pub fn get(&self, session_id: &SessionId) -> Option<&Session> {
        self.sessions.get(session_id)
    }

    pub fn get_mut(&mut self, session_id: &SessionId) -> Option<&mut Session> {
        self.sessions.get_mut(session_id)
    }

    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Provider display names from the registry (for dialog population).
    pub fn provider_names(&self) -> Vec<String> {
        self.registry.provider_names().into_iter().map(|s| s.to_string()).collect()
    }

    /// Default provider index from the registry (for dialog pre-selection).
    pub fn default_provider_index(&self) -> usize {
        self.registry.default_index()
    }

    /// Get the provider ID by index (for dialog submission).
    pub fn provider_id_at(&self, index: usize) -> Option<String> {
        self.registry.provider_ids().get(index).map(|s| s.to_string())
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn terminal_rows(&self) -> u16 {
        self.terminal_rows
    }

    /// Insert a session directly (for testing only).
    #[cfg(test)]
    pub fn insert_test_session(&mut self, session: Session) {
        self.sessions.insert(session.id.clone(), session);
    }

    /// Discover existing sessions from a previous TUI instance.
    ///
    /// Reads the provider id, working directory, and worktree path from tmux
    /// pane options (written at launch time), so cleanup on kill() targets
    /// the right provider and directories. Without these, a discovered
    /// session would fall back to a hardcoded "claude-code" provider and
    /// an empty working_dir, sending cleanup at the wrong place.
    pub async fn discover_existing(&mut self) -> Result<Vec<SessionId>> {
        if !commands::has_session(TMUX_SESSION_NAME).await? {
            return Ok(vec![]);
        }

        // Tab delimiter is safe even when a nickname contains spaces; a
        // working_dir containing a literal tab would break parsing, but
        // tabs in filesystem paths are extraordinarily rare.
        let format = "#{pane_id}\t#{window_name}\t#{@herald_session_id}\t#{@herald_provider_id}\t#{@herald_working_dir}\t#{@herald_worktree_path}\t#{pane_current_command}";
        let panes = commands::list_panes(TMUX_SESSION_NAME, format).await?;

        let mut discovered = Vec::new();
        for line in panes {
            let parts: Vec<&str> = line.splitn(7, '\t').collect();
            if parts.len() < 7 || parts[2].is_empty() {
                continue;
            }
            let pane_id = PaneId(parts[0].to_string());
            let nickname = parts[1].to_string();
            let session_id = SessionId(parts[2].to_string());
            let provider_id = parts[3];
            let working_dir_raw = parts[4];
            let worktree_raw = parts[5];
            let command = parts[6];

            if command == TMUX_SESSION_NAME {
                tracing::info!(pane_id = %pane_id, "skipping herald's own pane");
                continue;
            }

            // Legacy panes launched before these keys existed will report
            // empty strings; fall back to the old hardcoded defaults so
            // pre-existing sessions still discover, just without clean
            // worktree-removal support.
            let provider_id = if provider_id.is_empty() {
                "claude-code"
            } else {
                provider_id
            };
            let working_dir = if working_dir_raw.is_empty() {
                PathBuf::new()
            } else {
                PathBuf::from(working_dir_raw)
            };
            let worktree_path = if worktree_raw.is_empty() {
                None
            } else {
                Some(PathBuf::from(worktree_raw))
            };
            // Repo path is re-resolved from working_dir rather than stored
            // as a fourth pane option — one source of truth, and it
            // matches what launch() would have derived anyway.
            let repo_path = if worktree_path.is_some() && !working_dir.as_os_str().is_empty() {
                crate::worktree::git_toplevel(&working_dir).await.ok()
            } else {
                None
            };

            let mut session = Session::new(
                session_id.clone(),
                nickname,
                String::new(),
                working_dir,
                provider_id.to_string(),
            );
            session.tmux_pane_id = pane_id;
            session.worktree_path = worktree_path;
            session.repo_path = repo_path;
            self.sessions.insert(session_id.clone(), session);
            discovered.push(session_id);
        }

        Ok(discovered)
    }
}

/// Remove a partially-installed worktree during launch failure. Best-effort —
/// we log but do not propagate any removal error since the caller is about
/// to return the original failure anyway.
async fn rollback_worktree(worktree_path: &Option<PathBuf>, repo_path: &Option<PathBuf>) {
    if let (Some(wt), Some(repo)) = (worktree_path, repo_path) {
        if let Err(e) = crate::worktree::WorktreeManager::remove(repo, wt).await {
            tracing::warn!("failed to roll back worktree after launch failure: {}", e);
        }
    }
}

/// Roll back provider hooks that were installed for a session whose pane
/// creation failed. Best-effort — same rationale as `rollback_worktree`.
async fn rollback_hooks(
    registry: &Arc<ProviderRegistry>,
    provider_id: &str,
    session_id: &SessionId,
    effective_dir: &Path,
    runtime_dir: &Path,
) {
    let registry = registry.clone();
    let provider_id = provider_id.to_string();
    let sid = session_id.clone();
    let wd = effective_dir.to_path_buf();
    let rt_dir = runtime_dir.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(provider) = registry.get_by_id(&provider_id) {
            let ctx = HookSetupContext {
                session_id: &sid,
                working_dir: &wd,
                socket_path: rt_dir.join(format!("{}.sock", sid)),
                hook_script_path: rt_dir.join("herald-hook.py"),
            };
            let _ = provider.cleanup_hooks(&ctx);
        }
    })
    .await;
}
