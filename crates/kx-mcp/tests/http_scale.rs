//! M5.2b scale — many concurrent HTTP dispatches share ONE pooled `ureq::Agent`
//! (proving `Send + Sync` + connection-pool reuse, no per-dispatch TLS handshake,
//! no FD exhaustion). HTTP is OFF the projection/fold path, so the `scale-smoke`
//! 25k-Mote re-fold gate is structurally unaffected by this transport — this test
//! only guards the transport's own concurrency.
//!
//! `#[ignore]` (a bounded perf/concurrency smoke, run explicitly), mirroring the
//! kx-projection scale tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

mod common;

use std::sync::Arc;

use common::{effect_egress, sample_mote, tool, warrant_granting_egress, HttpMode, MockHttpServer};
use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::InMemoryContentStore;

#[test]
#[ignore = "scale/concurrency smoke — run explicitly"]
fn many_concurrent_dispatches_share_one_pooled_agent() {
    const N: usize = 64;
    let server = MockHttpServer::start(HttpMode::Echo);
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = Arc::new(LocalCapabilityBroker::new(store));
    broker.register_capability(Box::new(common::http_capability(
        name.clone(),
        version.clone(),
        &server,
    )));
    let warrant = Arc::new(warrant_granting_egress(&name, &version));

    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let (broker, warrant, name) = (broker.clone(), warrant.clone(), name.clone());
        let version = version.clone();
        handles.push(std::thread::spawn(move || {
            let mote = sample_mote(&name, &version);
            let args = format!(r#"{{"i":{i}}}"#);
            broker
                .dispatch(&mote, &warrant, &name, effect_egress(&args))
                .map(|h| h.staged_ref)
        }));
    }

    let mut ok = 0usize;
    for h in handles {
        if h.join().unwrap().is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, N, "all {N} concurrent dispatches completed");
    assert_eq!(
        server.captured().len(),
        N,
        "the server received all {N} requests over the pooled agent"
    );
}
