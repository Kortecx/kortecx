//! [`InMemoryAuditSink`] — a deterministic, in-memory [`AuditSink`] for tests and
//! embedding.
//!
//! Stores the typed [`AuditEvent`]s verbatim (no time, no hex rendering), so two
//! byte-identical runs produce equal [`Self::events`] — the deterministic assertion
//! surface. Lock poisoning is recovered (never propagated): a panicking auditor
//! thread must not crash a later `record`.

use std::sync::{Arc, Mutex, PoisonError};

use crate::event::AuditEvent;
use crate::sink::AuditSink;

/// A cheap, cloneable, thread-safe in-memory audit sink. Clones share one slot
/// (an `Arc<Mutex<…>>`), so the orchestrator and any reader observe the same log.
#[derive(Clone, Debug, Default)]
pub struct InMemoryAuditSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

impl InMemoryAuditSink {
    /// A new, empty in-memory sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// A point-in-time clone of the recorded events, in record order. The caller
    /// holds no lock across iteration. Lock poisoning is recovered.
    pub fn events(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Number of recorded events (cheap; avoids cloning).
    pub fn len(&self) -> usize {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// `true` when nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AuditSink for InMemoryAuditSink {
    fn record(&self, event: AuditEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

#[cfg(test)]
mod tests {
    use kx_content::ContentRef;
    use kx_mote::{MoteId, NdClass};

    use super::*;

    fn mid(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }

    #[test]
    fn records_in_order_and_counts() {
        let sink = InMemoryAuditSink::new();
        assert!(sink.is_empty());
        sink.record(AuditEvent::RunStarted { runnable: 8 });
        sink.record(AuditEvent::MoteCommitted {
            mote_id: mid(1),
            result_ref: ContentRef::from_bytes([2; 32]),
            nd_class: NdClass::Pure,
        });
        assert_eq!(sink.len(), 2);
        let events = sink.events();
        assert_eq!(events[0], AuditEvent::RunStarted { runnable: 8 });
        assert!(matches!(events[1], AuditEvent::MoteCommitted { .. }));
    }

    #[test]
    fn dropped_defaults_to_zero() {
        let sink = InMemoryAuditSink::new();
        assert_eq!(sink.dropped(), 0);
        sink.record(AuditEvent::RunStarted { runnable: 1 });
        assert_eq!(sink.dropped(), 0, "an in-memory sink never drops");
    }

    #[test]
    fn clone_shares_one_slot() {
        let sink = InMemoryAuditSink::new();
        let clone = sink.clone();
        clone.record(AuditEvent::RunStarted { runnable: 3 });
        assert_eq!(sink.len(), 1, "the original observes the clone's write");
    }

    #[test]
    fn poisoning_is_recovered() {
        let sink = InMemoryAuditSink::new();
        let clone = sink.clone();
        let _ = std::thread::spawn(move || {
            let _guard = clone.events.lock().unwrap();
            panic!("poison the audit lock");
        })
        .join();
        sink.record(AuditEvent::RunStarted { runnable: 1 });
        assert_eq!(sink.len(), 1, "poison recovered, never propagated");
    }
}
