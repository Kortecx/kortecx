//! Concurrency tests for `kx-normalizer` (SN-4 v2 #7).
//!
//! - Compile-time `Send + Sync` over the full public-type set.
//! - 4-thread thread-independence of `normalize_deterministic`
//!   (Arc<>'d input, byte-identical outputs across threads — pins the
//!   "no thread-local state" contract that machine-independent replay
//!   requires).

use std::sync::Arc;
use std::thread;

use kx_normalizer::{normalize_deterministic, NormalizerError, NormalizerKind, RuleSet};

// ---------------------------------------------------------------------------
// Compile-time Send + Sync
// ---------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_and_sync() {
    assert_send_sync::<RuleSet>();
    assert_send_sync::<NormalizerKind>();
    assert_send_sync::<NormalizerError>();
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence
// ---------------------------------------------------------------------------

#[test]
fn normalize_is_thread_independent_under_real_move() {
    let input = Arc::new(b"  ls   -la   foo\tbar  ".to_vec());

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let i = Arc::clone(&input);
            thread::spawn(move || {
                normalize_deterministic(RuleSet::CommandLineIntent, 1, &i).unwrap()
            })
        })
        .collect();

    let mut results = Vec::with_capacity(4);
    for h in handles {
        results.push(h.join().expect("worker did not panic"));
    }
    let first = &results[0];
    for r in &results[1..] {
        assert_eq!(
            first, r,
            "normalize_deterministic must be thread-independent (no thread-local state in the normalizer)"
        );
    }
    assert_eq!(&first[..], b"ls -la foo bar");
}

#[test]
fn unsupported_version_error_is_thread_independent() {
    // Pins that even the error path is deterministic across threads.
    let input = Arc::new(b"hi".to_vec());

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let i = Arc::clone(&input);
            thread::spawn(move || normalize_deterministic(RuleSet::CommandLineIntent, 999, &i))
        })
        .collect();

    let mut results = Vec::with_capacity(4);
    for h in handles {
        results.push(h.join().expect("worker did not panic"));
    }
    let first = &results[0];
    for r in &results[1..] {
        assert_eq!(first, r, "error path must be thread-independent");
    }
    assert!(matches!(
        first,
        Err(NormalizerError::UnsupportedVersion { .. })
    ));
}
