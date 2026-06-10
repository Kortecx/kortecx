//! The Morphic Data Engine capture read seam (the campaign Batch 2
//! `ListCaptureRecords` path).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`[u8; N]` / `String` /
//! `u64`) — NO `kx-capture` type crosses the seam, so gateway-core gains no
//! capture dependency and stays off the writer wall (its `dep_wall.rs` forbids
//! `kx-capture`). The host (`kx-gateway`) folds the journal into a durable
//! `capture.db` sidecar and implements [`CaptureView`] over it.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** Capture is a journal-DERIVED projection: a
//!   rebuildable cache, never journaled, never a `MoteId` input, never gating
//!   execution. Dropping it loses no truth (the journal + content store are
//!   truth).
//! - **Join-key-only (ActionsOnly).** A record carries the committed action's
//!   identity keys ONLY — `mote_id` / `instance_id` / `result_ref` / `nd_class`
//!   / `seq`, plus the ReAct `turn`/`branch` joined from the chain's metadata.
//!   No payload, reasoning, or thinking — the default privacy-safe scope, made
//!   structural here (the DTO has no such field).
//! - **`None` seam ⇒ `unimplemented`.** A gateway without the sidecar (an
//!   embedder, or `--catalog-dir`-less serve) degrades forward-compatibly.

/// One durably-captured ACTION record in a [`CaptureView::list`] enumeration —
/// a committed Mote's join keys (the always-retained ActionsOnly capture). The
/// `react_turn`/`react_branch` are set when the Mote is a ReAct turn (joined
/// from its `ReactRound` fact); empty/`None` otherwise.
#[derive(Clone, Debug)]
pub struct CaptureRecordEntry {
    /// The captured Mote's identity (the action's id).
    pub mote_id: [u8; 32],
    /// The run this action belongs to (single-node: the serve session).
    pub instance_id: [u8; 16],
    /// The committed action's content address — the truth join key
    /// (== the Mote's `result_ref` on the journal).
    pub result_ref: [u8; 32],
    /// `"pure"` | `"read_only_nondet"` | `"world_mutating"`.
    pub nd_class: String,
    /// The journal seq of the `Committed` fact (ordering / pagination cursor).
    pub seq: u64,
    /// The ReAct turn index, set iff the Mote is a coordinator-materialized
    /// ReAct turn (joined from its `ReactRound` fact).
    pub react_turn: Option<u32>,
    /// The ReAct turn's settled branch iff a ReAct turn (else empty).
    pub react_branch: String,
}

/// The capture read seam behind `ListCaptureRecords`. The host implements it
/// over its durable `capture.db` sidecar (the journal-derived action projection).
/// A `None` seam on the service ⇒ the RPC returns `unimplemented`.
pub trait CaptureView: Send + Sync {
    /// One newest-first page of captured action records, optionally scoped to one
    /// run's `instance_id`. `limit` is clamped to the host's max page (or the
    /// default when absent). Returns `(records, has_more)`.
    ///
    /// # Errors
    /// A host read failure ([`crate::error::GatewayError`]).
    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
    ) -> Result<(Vec<CaptureRecordEntry>, bool), crate::error::GatewayError>;
}
