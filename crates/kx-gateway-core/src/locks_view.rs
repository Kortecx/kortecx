//! POC-5b — the per-App lock seam behind `LockApp` / `UnlockApp` and the
//! `AdvanceBranch` write chokepoint.
//!
//! A lock is a per-party policy decision on an App's project BRANCH (keyed by the
//! branch handle — the same handle as the App in the one-App-one-branch model). When
//! a branch is locked the gateway REFUSES agentic in-CAS edits at the single
//! `AdvanceBranch` chokepoint (and inside the scaffold write loop), so the
//! agent-write surface is never reachable ungated.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** The `locks.db` sidecar is REBUILDABLE-TO-EMPTY: a lock
//!   is a policy decision, NOT journal-derivable. Never journaled, never a `MoteId`
//!   input, never a digest input — dropping the file cannot move the canonical
//!   projection digest.
//! - **Fails OPEN on loss.** A lock is an availability gate, not an integrity gate:
//!   if `locks.db` is lost / recreated empty, branches read as UNLOCKED (editing is
//!   restored), never bricked. This is the safe direction for an availability
//!   feature — documented so an operator knows losing the file unlocks.
//! - **Caller-scoped.** Every method takes the SERVER-RESOLVED `principal`; a party
//!   can only lock / unlock / observe its OWN branches.
//! - **`None` seam ⇒ degrade-open.** A host without the sidecar leaves `LockApp` /
//!   `UnlockApp` `unimplemented`, and the chokepoint treats the absent seam as
//!   "unlocked" (an additive feature can never tighten an existing serve).

use crate::error::GatewayError;

/// The structured refusal code emitted (as `kx-refusal-code` gRPC metadata) when an
/// `AdvanceBranch` is refused because the App's branch is locked. Clients act on the
/// CODE, never the prose (the PR-2 refusal-code contract).
pub const LOCKED_BRANCH_REFUSAL_CODE: &str = "LOCKED_BRANCH";

/// The per-App lock store seam: query / set / clear a caller's branch lock. A `None`
/// seam on the service ⇒ `LockApp` / `UnlockApp` are `unimplemented` and the
/// `AdvanceBranch` chokepoint degrades open (treats every branch as unlocked).
pub trait LockStore: Send + Sync {
    /// `true` iff `(principal, branch_handle)` is currently locked. A storage error
    /// surfaces as `GatewayError` (the chokepoint fails CLOSED on a real error — a
    /// query failure is NOT the degrade-open path; absence of the seam is).
    fn is_locked(&self, principal: &str, branch_handle: &str) -> Result<bool, GatewayError>;

    /// Lock `(principal, branch_handle)` (idempotent). Returns the post-state
    /// (`true` = locked).
    fn lock(&self, principal: &str, branch_handle: &str) -> Result<bool, GatewayError>;

    /// Unlock `(principal, branch_handle)` (idempotent). Returns `true` (= unlocked)
    /// on success — a no-op unlock of an already-unlocked branch still returns `true`.
    fn unlock(&self, principal: &str, branch_handle: &str) -> Result<bool, GatewayError>;
}
