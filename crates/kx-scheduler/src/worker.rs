//! [`WorkerId`] — opaque worker identifier returned by a [`crate::Placement`].

/// Opaque worker identifier returned by a [`crate::Placement`] implementation.
///
/// Semantic interpretation is the placement's; the scheduler treats it as a
/// black-box value and surfaces it back on [`crate::DispatchedMote::worker`].
/// The P1 single-process runtime uses `WorkerId(0)` for the local executor; a
/// multi-worker placement (P2) would issue distinct ids per remote worker.
///
/// ```
/// use kx_scheduler::WorkerId;
/// let w = WorkerId(7);
/// assert_eq!(w.0, 7);
/// // `Ord` is derived for use as a BTreeMap key.
/// assert!(WorkerId(1) < WorkerId(2));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerId(pub u64);
