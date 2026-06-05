//! Scale-smoke: the audit sink is flat per-event at scale.
//!
//! `#[ignore]`d — run in `--release` via the `scale-smoke` recipe. Unlike capture
//! (a once-at-end sweep), the audit sink is called on the hot drive loop at each
//! lifecycle transition, so this proves the per-event cost stays flat (no
//! super-linear path) for BOTH backends: the in-memory `Vec` push and the JSONL
//! buffered serialize+write. A regression (e.g. an O(n) rescan per event, or a
//! per-event fsync) would turn a large run super-linear and trip this gate.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use kx_audit::{AuditEvent, AuditSink, InMemoryAuditSink, JsonlAuditSink};
use kx_content::ContentRef;
use kx_mote::{MoteId, NdClass};

const SIZES: &[usize] = &[1_000, 5_000, 10_000, 25_000];

fn mote_at(i: usize) -> MoteId {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&(i as u64).to_le_bytes());
    MoteId::from_bytes(b)
}

fn committed(i: usize) -> AuditEvent {
    AuditEvent::MoteCommitted {
        mote_id: mote_at(i),
        result_ref: ContentRef::from_bytes([3u8; 32]),
        nd_class: NdClass::Pure,
    }
}

#[allow(clippy::cast_precision_loss)]
fn per_event_ratio(label: &str, sink: &dyn AuditSink, n_sizes: &[usize]) -> (f64, f64) {
    let mut per_event_ns: Vec<(usize, f64)> = Vec::new();
    for &n in n_sizes {
        let start = Instant::now();
        for i in 0..n {
            sink.record(committed(i));
        }
        sink.flush();
        let elapsed = start.elapsed();
        let per = elapsed.as_nanos() as f64 / n as f64;
        println!(
            "{label}: n={n} total_ms={} per_event_ns={per:.1}",
            elapsed.as_millis()
        );
        per_event_ns.push((n, per));
    }
    (
        per_event_ns.first().unwrap().1,
        per_event_ns.last().unwrap().1,
    )
}

#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn audit_sink_is_flat_per_event() {
    // In-memory: a `Vec` push is amortized O(1); the 25k/1k ratio should be ~1×.
    let mem = InMemoryAuditSink::new();
    let (mem_first, mem_last) = per_event_ratio("audit-in-memory", &mem, SIZES);
    assert!(
        mem_last <= mem_first * 4.0,
        "in-memory per-event must stay flat (1k {mem_first:.1}ns vs 25k {mem_last:.1}ns)"
    );

    // JSONL: serialize + buffered write per event (NO per-event fsync), so the
    // per-event cost is bounded and flat across sizes (a quadratic regression ≈ 25×).
    let dir = tempfile::tempdir().unwrap();
    let jsonl = JsonlAuditSink::create(dir.path().join("audit.jsonl")).unwrap();
    let (j_first, j_last) = per_event_ratio("audit-jsonl", &jsonl, SIZES);
    assert!(
        j_last <= j_first * 4.0,
        "jsonl per-event must stay flat (1k {j_first:.1}ns vs 25k {j_last:.1}ns)"
    );
    assert_eq!(jsonl.dropped(), 0, "no events dropped at scale");
}
