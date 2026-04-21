use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::session::model::SessionId;

/// Manages git worktree lifecycle for session isolation.
///
/// Worktrees are stored outside the source repo at
/// `<data-dir>/herald/worktrees/<repo-basename>/<sanitized-name>-<short-uuid>/`
/// so the source repo stays clean and no `.gitignore` entries are needed.
/// The caller passes both `repo_path` (the git repository root) and
/// `worktree_path` explicitly to `remove`/`cleanup_orphaned` — we never
/// infer one from the other.
pub struct WorktreeManager;

impl WorktreeManager {
    /// Create a git worktree for a session.
    ///
    /// `repo_path` must be either the git repository root or any path inside
    /// the repo; we resolve to the canonical toplevel before deriving the
    /// basename used in the worktree's storage path.
    ///
    /// Branch: `herald/<sanitized-nickname>-<short-uuid>`.
    /// Worktree: `<herald_worktrees_root>/<repo-basename>/<sanitized-nickname>-<short-uuid>/`.
    ///
    /// Returns the absolute path to the new worktree directory.
    pub async fn create(
        repo_path: &Path,
        nickname: &str,
        session_id: &SessionId,
    ) -> Result<PathBuf> {
        let repo_root = git_toplevel(repo_path).await?;
        let short_id = &session_id.as_str()[..8.min(session_id.as_str().len())];
        let name = format!("{}-{}", sanitize_name(nickname), short_id);
        let branch = format!("herald/{}", name);

        let worktree_path = herald_worktrees_root()?
            .join(repo_basename(&repo_root)?)
            .join(&name);

        if let Some(parent) = worktree_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("creating worktree parent directory")?;
        }

        let output = Command::new("git")
            .args(["worktree", "add", "-b", &branch])
            .arg(&worktree_path)
            .arg("HEAD")
            .current_dir(&repo_root)
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
    ///
    /// Both `repo_path` and `worktree_path` must be supplied — we do not
    /// infer the repo from the worktree's parent chain.
    pub async fn remove(repo_path: &Path, worktree_path: &Path) -> Result<()> {
        let branch_name = worktree_path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|name| format!("herald/{}", name));

        let output = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(worktree_path)
            .current_dir(repo_path)
            .output()
            .await
            .context("running git worktree remove")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("git worktree remove failed: {}", stderr.trim());
            let _ = tokio::fs::remove_dir_all(worktree_path).await;
            let _ = Command::new("git")
                .args(["worktree", "prune"])
                .current_dir(repo_path)
                .output()
                .await;
        }

        if let Some(branch) = branch_name {
            let _ = Command::new("git")
                .args(["branch", "-D", &branch])
                .current_dir(repo_path)
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

    /// Clean up orphaned Herald worktrees under the repo's storage dir that
    /// don't match any active session.
    #[cfg(test)]
    pub async fn cleanup_orphaned(
        repo_path: &Path,
        active_ids: &[SessionId],
    ) -> Result<()> {
        let repo_root = git_toplevel(repo_path).await?;
        let worktree_dir = herald_worktrees_root()?.join(repo_basename(&repo_root)?);
        if !worktree_dir.exists() {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(&worktree_dir)
            .await
            .context("reading herald worktrees dir")?;

        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_active = active_ids.iter().any(|id| {
                let short_id = &id.as_str()[..8.min(id.as_str().len())];
                name.ends_with(short_id)
            });

            if !is_active {
                tracing::info!(worktree = %name, "cleaning up orphaned worktree");
                let _ = Self::remove(&repo_root, &entry.path()).await;
            }
        }

        Ok(())
    }
}

/// Resolve any path inside a git repo to the canonical repository root via
/// `git rev-parse --show-toplevel`.
pub async fn git_toplevel(dir: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .await
        .context("running git rev-parse --show-toplevel")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "not a git repository ({}): {}",
            dir.display(),
            stderr.trim()
        );
    }

    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path_str.is_empty() {
        anyhow::bail!("git rev-parse returned empty path for {}", dir.display());
    }
    Ok(PathBuf::from(path_str))
}

/// Root directory where Herald stores all worktrees across repos.
/// Matches the data-dir convention used for logs (`dirs::data_dir()/herald/...`).
fn herald_worktrees_root() -> Result<PathBuf> {
    let base = dirs::data_dir()
        .context("no platform data directory available")?;
    Ok(base.join("herald").join("worktrees"))
}

/// Extract the basename of a repo path for use as a storage subdirectory,
/// stripping a trailing `.git` if present (matches Orca's convention).
fn repo_basename(repo_path: &Path) -> Result<String> {
    let base = repo_path
        .file_name()
        .and_then(|s| s.to_str())
        .context("repo path has no basename")?;
    Ok(base.trim_end_matches(".git").to_string())
}

/// Sanitize a nickname for use in branch/directory names.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
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

    /// Serialize tests that mutate the global env vars read by
    /// `dirs::data_dir()` — otherwise one test's `set_var` races with
    /// another's reads and they observe each other's data dirs.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Override the data dir for tests and hold a lock for the test's
    /// duration. Tests must bind the returned guard: `let _env = ...`.
    fn override_data_dir(tmp: &Path) -> std::sync::MutexGuard<'static, ()> {
        let guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        #[cfg(target_os = "macos")]
        {
            std::env::set_var("HOME", tmp);
        }
        #[cfg(not(target_os = "macos"))]
        {
            std::env::set_var("XDG_DATA_HOME", tmp);
        }
        guard
    }

    #[tokio::test]
    async fn create_places_worktree_outside_repo() {
        let (_dir, repo) = make_test_repo().await;
        let data_home = tempfile::tempdir().unwrap();
        let _env = override_data_dir(data_home.path());

        let session_id = SessionId("abcdef12-3456-7890-abcd-ef1234567890".into());
        let wt = WorktreeManager::create(&repo, "test-session", &session_id)
            .await
            .unwrap();

        assert!(wt.exists());
        assert!(wt.join(".git").exists());
        // Critical: worktree must NOT be inside the source repo.
        assert!(!wt.starts_with(&repo));
        // And it must live under <data>/herald/worktrees/<repo-basename>/
        let worktrees_root = herald_worktrees_root().unwrap();
        assert!(wt.starts_with(&worktrees_root));

        WorktreeManager::remove(&repo, &wt).await.unwrap();
        assert!(!wt.exists());
    }

    #[tokio::test]
    async fn create_accepts_subdirectory_of_repo() {
        // Regression guard: if a user launches Herald from a subdir,
        // create() must resolve to the repo root, not treat the subdir as root.
        let (_dir, repo) = make_test_repo().await;
        let data_home = tempfile::tempdir().unwrap();
        let _env = override_data_dir(data_home.path());

        let subdir = repo.join("src");
        tokio::fs::create_dir_all(&subdir).await.unwrap();

        let session_id = SessionId("subdir01-3456-7890-abcd-ef1234567890".into());
        let wt = WorktreeManager::create(&subdir, "from-subdir", &session_id)
            .await
            .unwrap();

        // Storage basename should match the repo root, not the subdir.
        let repo_basename_str = repo.file_name().unwrap().to_string_lossy().to_string();
        assert!(wt
            .to_string_lossy()
            .contains(&format!("worktrees/{}/", repo_basename_str)));

        WorktreeManager::remove(&repo, &wt).await.unwrap();
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
        let data_home = tempfile::tempdir().unwrap();
        let _env = override_data_dir(data_home.path());

        let sid1 = SessionId("aaaaaaaa-1111-2222-3333-444444444444".into());
        let sid2 = SessionId("bbbbbbbb-1111-2222-3333-444444444444".into());

        let wt1 = WorktreeManager::create(&repo, "sess1", &sid1).await.unwrap();
        let wt2 = WorktreeManager::create(&repo, "sess2", &sid2).await.unwrap();
        assert!(wt1.exists());
        assert!(wt2.exists());

        WorktreeManager::cleanup_orphaned(&repo, &[sid1.clone()])
            .await
            .unwrap();
        assert!(wt1.exists());
        assert!(!wt2.exists());

        WorktreeManager::remove(&repo, &wt1).await.unwrap();
    }

    #[tokio::test]
    async fn sanitize_name_replaces_special_chars() {
        assert_eq!(sanitize_name("my session!"), "my-session-");
        assert_eq!(sanitize_name("fix/tests"), "fix-tests");
        assert_eq!(sanitize_name("good-name_1"), "good-name_1");
    }

    #[tokio::test]
    async fn repo_basename_strips_dot_git() {
        assert_eq!(
            repo_basename(Path::new("/some/where/herald")).unwrap(),
            "herald"
        );
        assert_eq!(
            repo_basename(Path::new("/some/where/herald.git")).unwrap(),
            "herald"
        );
    }

    #[tokio::test]
    async fn git_toplevel_resolves_from_subdirectory() {
        let (_dir, repo) = make_test_repo().await;
        let subdir = repo.join("deep").join("nested");
        tokio::fs::create_dir_all(&subdir).await.unwrap();

        let resolved = git_toplevel(&subdir).await.unwrap();
        // git rev-parse may canonicalize symlinks (macOS /var → /private/var),
        // so compare via canonicalize on both sides.
        assert_eq!(
            resolved.canonicalize().unwrap(),
            repo.canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn git_toplevel_fails_outside_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(git_toplevel(dir.path()).await.is_err());
    }
}
