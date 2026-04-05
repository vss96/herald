use std::time::{Duration, Instant};

/// A flushed batch of literal keystrokes for a specific tmux pane.
pub struct PaneBatch {
    pub pane_id: String,
    pub text: String,
}

/// Batches literal keystrokes destined for a tmux pane, flushing them
/// as a single `send-keys -l` call after a short delay.
pub struct KeyBatcher {
    pane_id: String,
    buffer: String,
    batch_start: Option<Instant>,
}

const BATCH_DELAY: Duration = Duration::from_millis(8);

impl KeyBatcher {
    pub fn new() -> Self {
        Self {
            pane_id: String::new(),
            buffer: String::new(),
            batch_start: None,
        }
    }

    /// Add a literal character to the batch. If the pane changed, returns
    /// the old batch that should be flushed first.
    pub fn push_literal(&mut self, pane_id: &str, text: &str) -> Option<PaneBatch> {
        let stale = if !self.buffer.is_empty() && self.pane_id != pane_id {
            Some(self.take_batch())
        } else {
            None
        };

        if self.buffer.is_empty() {
            self.pane_id = pane_id.to_string();
            self.batch_start = Some(Instant::now());
        }
        self.buffer.push_str(text);
        stale
    }

    /// Take the current batch out, or `None` if empty.
    pub fn take(&mut self) -> Option<PaneBatch> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(self.take_batch())
        }
    }

    /// When the current batch should be flushed, or `None` if empty.
    pub fn flush_deadline(&self) -> Option<Instant> {
        self.batch_start.map(|t| t + BATCH_DELAY)
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    fn take_batch(&mut self) -> PaneBatch {
        let pane_id = std::mem::take(&mut self.pane_id);
        let text = std::mem::take(&mut self.buffer);
        self.batch_start = None;
        PaneBatch { pane_id, text }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_by_default() {
        let b = KeyBatcher::new();
        assert!(b.is_empty());
        assert!(b.flush_deadline().is_none());
    }

    #[test]
    fn push_and_take() {
        let mut b = KeyBatcher::new();
        assert!(b.push_literal("%1", "h").is_none());
        assert!(b.push_literal("%1", "i").is_none());
        assert!(!b.is_empty());

        let batch = b.take().unwrap();
        assert_eq!(batch.pane_id, "%1");
        assert_eq!(batch.text, "hi");
        assert!(b.is_empty());
    }

    #[test]
    fn pane_change_flushes_old() {
        let mut b = KeyBatcher::new();
        b.push_literal("%1", "a");
        let stale = b.push_literal("%2", "b").unwrap();
        assert_eq!(stale.pane_id, "%1");
        assert_eq!(stale.text, "a");

        let batch = b.take().unwrap();
        assert_eq!(batch.pane_id, "%2");
        assert_eq!(batch.text, "b");
    }

    #[test]
    fn flush_deadline_set_on_first_push() {
        let mut b = KeyBatcher::new();
        let before = Instant::now();
        b.push_literal("%1", "x");
        let deadline = b.flush_deadline().unwrap();
        assert!(deadline >= before + BATCH_DELAY);
        assert!(deadline <= Instant::now() + BATCH_DELAY);
    }

    #[test]
    fn take_on_empty_returns_none() {
        let mut b = KeyBatcher::new();
        assert!(b.take().is_none());
    }
}
