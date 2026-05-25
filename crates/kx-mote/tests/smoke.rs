// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! P1.1 workspace smoke test.
//!
//! Verifies the workspace's tracing setup is wired correctly by initializing a
//! `tracing-subscriber` and emitting one structured line. The presence of this
//! line in CI's output satisfies the P1.1 exit gate per `01-build-sequence.md` §1.1
//! ("`tracing` emits a structured line").
//!
//! P1.2 will replace this with the real Mote unit tests.

#[test]
fn workspace_tracing_smoke() {
    // try_init returns Err if a subscriber is already registered; that's fine
    // because other tests in the same process may have set one. Either way we
    // proceed to emit the structured line.
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    tracing::info!(
        crate_name = "kx-mote",
        phase = "P1.1",
        gate = "workspace-skeleton",
        "workspace skeleton smoke test"
    );
}
