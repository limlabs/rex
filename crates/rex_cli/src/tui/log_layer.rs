use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: Level,
    pub message: String,
}

/// Thread-safe ring buffer of log entries.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    total_count: Arc<Mutex<usize>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            total_count: Arc::new(Mutex::new(0)),
            capacity,
        }
    }

    pub fn push(&self, entry: LogEntry) {
        if let Ok(mut buf) = self.inner.lock() {
            if buf.len() >= self.capacity {
                buf.pop_front();
            }
            buf.push_back(entry);
        }
        if let Ok(mut count) = self.total_count.lock() {
            *count += 1;
        }
    }

    /// Return a snapshot of all entries.
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner
            .lock()
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Return the most recent entry, if any.
    pub fn last(&self) -> Option<LogEntry> {
        self.inner
            .lock()
            .map(|buf| buf.back().cloned())
            .unwrap_or(None)
    }

    /// Return total number of entries ever pushed (monotonically increasing).
    pub fn total_count(&self) -> usize {
        self.total_count.lock().map(|c| *c).unwrap_or(0)
    }

    /// Return entries added since `since_count` (based on total_count values).
    /// Returns the new entries and the updated total_count.
    pub fn drain_since(&self, since_count: usize) -> (Vec<LogEntry>, usize) {
        let total = self.total_count();
        if total <= since_count {
            return (Vec::new(), since_count);
        }
        let new_count = total - since_count;
        let entries = self
            .inner
            .lock()
            .map(|buf| {
                let buf_len = buf.len();
                let skip = buf_len.saturating_sub(new_count);
                buf.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_default();
        (entries, total)
    }
}

/// Extracts the message and fields from a tracing event.
struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: Vec::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}

/// A tracing Layer that captures log events into a LogBuffer.
pub struct TuiLogLayer {
    buffer: LogBuffer,
}

impl TuiLogLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: Subscriber> Layer<S> for TuiLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        let mut message = visitor.message;
        if !visitor.fields.is_empty() {
            message.push(' ');
            message.push_str(&visitor.fields.join(" "));
        }

        self.buffer.push(LogEntry {
            level: *event.metadata().level(),
            message,
        });
    }
}
