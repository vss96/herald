use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::io::AsyncBufReadExt;
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use crate::events::types::HookEvent;

/// Listens on a per-session Unix socket for hook events.
pub struct HookListener {
    socket_path: PathBuf,
    buffer_path: PathBuf,
}

impl HookListener {
    pub fn new(runtime_dir: &Path, session_id: &str) -> Self {
        Self {
            socket_path: runtime_dir.join(format!("{}.sock", session_id)),
            buffer_path: runtime_dir.join(format!("{}.buffer", session_id)),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn buffer_path(&self) -> &Path {
        &self.buffer_path
    }

    /// Drain buffered events from the buffer file (for recovery after restart).
    ///
    /// Uses atomic rename-then-read to avoid racing with concurrent hook writes.
    /// The hook script uses flock on the buffer, so we rename the file first
    /// (atomic on the same filesystem), then read from the renamed copy.
    /// New hook writes will create a fresh buffer file.
    pub async fn drain_buffer(&self) -> Result<Vec<HookEvent>> {
        let drain_path = self.buffer_path.with_extension("draining");

        // Atomically rename buffer -> draining (new writes go to a fresh buffer)
        match tokio::fs::rename(&self.buffer_path, &drain_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e).context("renaming buffer file for drain"),
        }

        // Read from the renamed file (no concurrent writers)
        let content = tokio::fs::read_to_string(&drain_path)
            .await
            .context("reading draining buffer")?;

        // Clean up the drain file
        let _ = tokio::fs::remove_file(&drain_path).await;

        let events: Vec<HookEvent> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        Ok(events)
    }

    /// Start listening on the Unix socket, sending events to the channel.
    pub async fn listen(&self, tx: mpsc::Sender<HookEvent>) -> Result<()> {
        // Remove stale socket file
        let _ = tokio::fs::remove_file(&self.socket_path).await;

        let listener = UnixListener::bind(&self.socket_path)
            .context("binding Unix socket")?;

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let reader = tokio::io::BufReader::new(stream);
                        let mut lines = reader.lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            if let Ok(event) = serde_json::from_str::<HookEvent>(&line) {
                                if let Err(e) = tx.send(event).await {
                                    tracing::warn!("hook event channel send failed (receiver dropped): {}", e);
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!("socket accept error: {}", e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::types::HookEventName;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixStream;

    #[tokio::test]
    async fn drain_buffer_parses_events() {
        let dir = tempfile::tempdir().unwrap();
        let listener = HookListener::new(dir.path(), "test-session");

        // Write some buffered events
        let events = vec![
            r#"{"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Edit"}"#,
            r#"{"session_id":"s1","hook_event_name":"Stop"}"#,
        ];
        tokio::fs::write(&listener.buffer_path, events.join("\n"))
            .await
            .unwrap();

        let drained = listener.drain_buffer().await.unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].hook_event_name, HookEventName::PermissionRequest);
        assert_eq!(drained[1].hook_event_name, HookEventName::Stop);

        // Original buffer file should be gone (renamed and cleaned up)
        assert!(!listener.buffer_path.exists());
        // Drain file should also be cleaned up
        assert!(!listener.buffer_path.with_extension("draining").exists());
    }

    #[tokio::test]
    async fn drain_buffer_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let listener = HookListener::new(dir.path(), "nonexistent");
        let drained = listener.drain_buffer().await.unwrap();
        assert!(drained.is_empty());
    }

    #[tokio::test]
    async fn drain_buffer_skips_malformed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let listener = HookListener::new(dir.path(), "test-session");

        let content = [
            r#"{"session_id":"s1","hook_event_name":"Stop"}"#,
            "not valid json",
            r#"{"session_id":"s1","hook_event_name":"Notification"}"#,
        ]
        .join("\n");
        tokio::fs::write(&listener.buffer_path, content)
            .await
            .unwrap();

        let drained = listener.drain_buffer().await.unwrap();
        assert_eq!(drained.len(), 2);
    }

    #[tokio::test]
    async fn socket_listener_receives_events() {
        let dir = tempfile::tempdir().unwrap();
        let listener = HookListener::new(dir.path(), "test-session");

        let (tx, mut rx) = mpsc::channel(10);

        // Start listener in background
        let socket_path = listener.socket_path().to_path_buf();
        tokio::spawn(async move {
            listener.listen(tx).await.unwrap();
        });

        // Give listener time to bind
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect and send an event
        let mut stream = UnixStream::connect(&socket_path).await.unwrap();
        let event_json =
            r#"{"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Edit"}"#;
        stream
            .write_all(format!("{}\n", event_json).as_bytes())
            .await
            .unwrap();
        stream.shutdown().await.unwrap();

        // Receive the event
        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.session_id, "s1");
        assert_eq!(event.hook_event_name, HookEventName::PermissionRequest);
    }
}
