use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::session::model::SessionId;

/// Directory within a repo root where Herald stores its worktrees.
const WORKTREE_DIR: &str = ".herald-worktrees";

/// Manages git worktree lifecycle for session isolation.
pub struct WorktreeManager;

impl WorktreeManager {
    /// Create a git worktree for a session.
    ///
    /// Location: `<repo-root>/.herald-worktrees/<nickname>-<short-uuid>/`
    /// Branch:   `herald/<nickname>-<short-uuid>`
    ///
    /// Returns the absolute path to the new worktree directory.
    pub async fn create(
        repo_dir: &Path,
        nickname: &str,
        session_id: &SessionId,
    ) -> Result<PathBuf> {
        let short_id = &session_id.as_str()[..8.min(session_id.as_str().len())];
        let name = format!("{}-{}", sanitize_name(nickname), short_id);
        let branch = format!("herald/{}", name);
        let worktree_path = repo_dir.join(WORKTREE_DIR).join(&name);

        let output = Command::new("git")
            .args(["worktree", "add", "-b", &branch])
            .arg(&worktree_path)
            .arg("HEAD")
            .current_dir(repo_dir)
            .output()
            .await
            .context("running git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git worktree add failed: {}", stderr.trim());
        }

        Ok(worktree_path)
    }

    /// Remove a git worktree and its associated branch. Best-effort.
    pub async fn remove(worktree_path: &Path) -> Result<()> {
        // Extract repo root from the worktree path
        // Worktree is at <repo>/.herald-worktrees/<name>, so repo is 2 levels up
        let repo_dir = worktree_path
            .parent()
            .and_then(|p| p.parent())
            .context("cannot determine repo root from worktree path")?;

        // Try to extract the branch name from .git file in the worktree
        let branch_name = read_worktree_branch(worktree_path).await;

        // Remove the worktree
        let output = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(worktree_path)
            .current_dir(repo_dir)
            .output()
            .await
            .context("running git worktree remove")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("git worktree remove failed: {}", stderr.trim());
            // Fall back to manual cleanup
            let _ = tokio::fs::remove_dir_all(worktree_path).await;
            // Prune stale worktree entries
            let _ = Command::new("git")
                .args(["worktree", "prune"])
                .current_dir(repo_dir)
                .output()
                .await;
        }

        // Delete the herald branch (best-effort)
        if let Some(branch) = branch_name {
            let _ = Command::new("git")
                .args(["branch", "-D", &branch])
                .current_dir(repo_dir)
                .output()
                .await;
        }

        Ok(())
    }

    /// Check if a directory is inside a git repo with at least one commit (async).
    #[cfg(test)]
    pub async fn can_create_worktree(dir: &Path) -> bool {
        if dir.as_os_str().is_empty() {
            return false;
        }
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Synchronous version of `can_create_worktree` (for dialog initialization).
    pub fn can_create_worktree_sync(dir: &Path) -> bool {
        if dir.as_os_str().is_empty() {
            return false;
        }
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Clean up orphaned Herald worktrees not matching any active session.
    #[cfg(test)]
    pub async fn cleanup_orphaned(
        repo_dir: &Path,
        active_ids: &[SessionId],
    ) -> Result<()> {
        let worktree_dir = repo_dir.join(WORKTREE_DIR);
        if !worktree_dir.exists() {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(&worktree_dir)
            .await
            .context("reading .herald-worktrees")?;

        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_active = active_ids.iter().any(|id| {
                let short_id = &id.as_str()[..8.min(id.as_str().len())];
                name.ends_with(short_id)
            });

            if !is_active {
                tracing::info!(worktree = %name, "cleaning up orphaned worktree");
                let _ = Self::remove(&entry.path()).await;
            }
        }

        Ok(())
    }
}

/// Sanitize a nickname for use in branch/directory names.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

/// Try to read the branch name from a worktree's .git file.
async fn read_worktree_branch(worktree_path: &Path) -> Option<String> {
    // The worktree name follows herald/ convention
    let name = worktree_path.file_name()?.to_string_lossy();
    Some(format!("herald/{}", name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Create a temporary git repo with an initial commit for testing.
    async fn make_test_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();

        Command::new("git")
            .args(["init"])
            .current_dir(&repo)
            .output()
            .await
            .unwrap();
        // Configure identity for CI environments where global git config is absent
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo)
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo)
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(&repo)
            .output()
            .await
            .unwrap();

        (dir, repo)
    }

    #[tokio::test]
    async fn create_and_remove_worktree() {
        let (_dir, repo) = make_test_repo().await;
        let session_id = SessionId("abcdef12-3456-7890-abcd-ef1234567890".into());

        let wt = WorktreeManager::create(&repo, "test-session", &session_id).await.unwrap();
        assert!(wt.exists());
        assert!(wt.join(".git").exists()); // worktree has a .git file

        WorktreeManager::remove(&wt).await.unwrap();
        assert!(!wt.exists());
    }

    #[tokio::test]
    async fn can_create_worktree_in_git_repo() {
        let (_dir, repo) = make_test_repo().await;
        assert!(WorktreeManager::can_create_worktree(&repo).await);
    }

    #[tokio::test]
    async fn cannot_create_worktree_outside_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!WorktreeManager::can_create_worktree(dir.path()).await);
    }

    #[tokio::test]
    async fn cannot_create_worktree_with_empty_path() {
        assert!(!WorktreeManager::can_create_worktree(Path::new("")).await);
    }

    #[tokio::test]
    async fn cleanup_orphaned_removes_stale_worktrees() {
        let (_dir, repo) = make_test_repo().await;
        let sid1 = SessionId("aaaaaaaa-1111-2222-3333-444444444444".into());
        let sid2 = SessionId("bbbbbbbb-1111-2222-3333-444444444444".into());

        let wt1 = WorktreeManager::create(&repo, "sess1", &sid1).await.unwrap();
        let wt2 = WorktreeManager::create(&repo, "sess2", &sid2).await.unwrap();
        assert!(wt1.exists());
        assert!(wt2.exists());

        // Only sid1 is active — sid2 should be cleaned up
        WorktreeManager::cleanup_orphaned(&repo, &[sid1.clone()]).await.unwrap();
        assert!(wt1.exists());
        assert!(!wt2.exists());

        // Clean up
        WorktreeManager::remove(&wt1).await.unwrap();
    }

    #[tokio::test]
    async fn sanitize_name_replaces_special_chars() {
        assert_eq!(sanitize_name("my session!"), "my-session-");
        assert_eq!(sanitize_name("fix/tests"), "fix-tests");
        assert_eq!(sanitize_name("good-name_1"), "good-name_1");
    }
}
