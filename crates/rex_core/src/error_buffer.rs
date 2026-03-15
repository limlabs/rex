use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize)]
pub struct DevError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub kind: String,
    pub timestamp: u64,
}

/// Thread-safe ring buffer of recent dev errors.
///
/// Errors are pushed on build/tsc failures and cleared on successful rebuilds.
/// The MCP server reads snapshots via the `/_rex/dev/errors` endpoint.
#[derive(Clone)]
pub struct ErrorBuffer {
    inner: Arc<Mutex<VecDeque<DevError>>>,
    capacity: usize,
}

impl ErrorBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    pub fn push(&self, error: DevError) {
        let mut buf = self.inner.lock().expect("ErrorBuffer lock poisoned");
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(error);
    }

    pub fn clear(&self) {
        let mut buf = self.inner.lock().expect("ErrorBuffer lock poisoned");
        buf.clear();
    }

    pub fn snapshot(&self) -> Vec<DevError> {
        let buf = self.inner.lock().expect("ErrorBuffer lock poisoned");
        buf.iter().cloned().collect()
    }

    pub fn push_build_error(&self, message: &str, file: Option<&str>) {
        self.push(DevError {
            message: message.to_string(),
            file: file.map(|s| s.to_string()),
            kind: "build".to_string(),
            timestamp: now_epoch_secs(),
        });
    }

    pub fn push_server_error(&self, message: &str, file: Option<&str>) {
        self.push(DevError {
            message: message.to_string(),
            file: file.map(|s| s.to_string()),
            kind: "server".to_string(),
            timestamp: now_epoch_secs(),
        });
    }

    pub fn push_tsc_error(&self, message: &str, file: &str) {
        self.push(DevError {
            message: message.to_string(),
            file: Some(file.to_string()),
            kind: "typescript".to_string(),
            timestamp: now_epoch_secs(),
        });
    }
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn push_and_snapshot() {
        let buf = ErrorBuffer::new(4);
        buf.push_build_error("err1", Some("a.ts"));
        buf.push_server_error("err2", None);
        let snap = buf.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].kind, "build");
        assert_eq!(snap[1].kind, "server");
    }

    #[test]
    fn capacity_evicts_oldest() {
        let buf = ErrorBuffer::new(2);
        buf.push_build_error("first", None);
        buf.push_build_error("second", None);
        buf.push_build_error("third", None);
        let snap = buf.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].message, "second");
        assert_eq!(snap[1].message, "third");
    }

    #[test]
    fn clear_removes_all() {
        let buf = ErrorBuffer::new(8);
        buf.push_build_error("err", None);
        buf.push_tsc_error("tsc err", "file.ts");
        assert_eq!(buf.snapshot().len(), 2);
        buf.clear();
        assert!(buf.snapshot().is_empty());
    }
}
