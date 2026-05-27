use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

// ── shared log state ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub message: String,
    pub is_error: bool,
}

#[derive(Clone)]
pub struct AppLog {
    entries: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl AppLog {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn last(&self) -> Option<LogEntry> {
        self.entries.lock().unwrap().back().cloned()
    }

    fn push(&self, message: String, is_error: bool) {
        let mut q = self.entries.lock().unwrap();
        q.push_back(LogEntry { message, is_error });
        if q.len() > 500 {
            q.pop_front();
        }
    }
}

// ── tracing layer ─────────────────────────────────────────────────────────

pub struct AppLogLayer {
    log: AppLog,
}

impl AppLogLayer {
    pub fn new(log: AppLog) -> Self {
        Self { log }
    }
}

impl<S: tracing::Subscriber> Layer<S> for AppLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let level = event.metadata().level();
        let is_error = *level == tracing::Level::ERROR || *level == tracing::Level::WARN;

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        if let Some(msg) = visitor.message {
            self.log.push(msg, is_error);
        }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" && self.message.is_none() {
            self.message = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" && self.message.is_none() {
            // format_args!(...) formats identically with {:?} as with {} for Arguments
            self.message = Some(format!("{value:?}"));
        }
    }
}
