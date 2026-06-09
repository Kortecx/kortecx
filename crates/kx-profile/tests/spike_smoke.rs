//! End-to-end witness that the profiling harness runs against a REAL in-process
//! gateway: one iteration produces one finite warm-up + one finite
//! submit→Committed sample. (Mirrors the `kx-cli` client e2e shape — hosts the
//! gateway in-process, drives it over the gRPC client.)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_profile::spikes;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn measure_produces_finite_samples() {
    let samples = spikes::measure(1)
        .await
        .expect("one profiling iteration over a fresh in-process gateway");

    assert_eq!(samples.warmup_ms.len(), 1, "one warm-up sample");
    assert_eq!(samples.submit_ms.len(), 1, "one submit→Committed sample");

    let warmup = samples.warmup_ms[0];
    let submit = samples.submit_ms[0];
    assert!(
        warmup.is_finite() && warmup >= 0.0,
        "warm-up {warmup} is sane"
    );
    assert!(
        submit.is_finite() && submit >= 0.0,
        "submit {submit} is sane"
    );
}
