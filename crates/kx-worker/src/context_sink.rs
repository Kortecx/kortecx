//! F-7 (assemble-into-serve) — the seam by which the worker hands a leased Mote's
//! resolved Data context (`WorkItem.parent_results`) to its executor.
//!
//! The frozen `kx_executor::MoteExecutor::run(&self, mote, warrant, env)` carries
//! no snapshot, so a model executor that wants to assemble upstream context cannot
//! receive it through the trait method. Instead the worker sets the context on this
//! side-channel *before* each dispatch; the gateway's model executor implements
//! [`ContextSink`] and reads the slot inside `run`. The frozen trait is untouched.
//!
//! The worker processes a lease batch sequentially on one thread, so a single slot
//! keyed by `MoteId` is sufficient and race-free: `set_parent_results` immediately
//! precedes the executor's `run`, which consumes the slot iff it matches the Mote.
//! A worker whose executor does not assemble simply holds no sink (`None`).

use kx_content::ContentRef;
use kx_mote::MoteId;

/// The worker → executor F-7 context side-channel. Implemented by an executor that
/// assembles upstream context for a model Mote (the gateway's `ModelRouterExecutor`).
///
/// `Send + Sync` so the worker can hold an `Arc<dyn ContextSink>` alongside its
/// `Arc<dyn MoteExecutor>` (the gateway clones ONE `Arc` into both roles).
pub trait ContextSink: Send + Sync {
    /// Stash the leased Mote's resolved Data context (its committed
    /// `(parent MoteId, result_ref)` pairs) for the executor to consume on the next
    /// `run` of `mote_id`. An empty list (the common case for a non-model or
    /// no-Data-context Mote) is delivered too, so a stale prior slot can never leak
    /// into the wrong Mote.
    fn set_parent_results(&self, mote_id: MoteId, parents: Vec<(MoteId, ContentRef)>);

    /// PR-9d (per-turn upstream context-carry): stash the run's grounding-context
    /// bundle ref for a SUCCESSOR ReAct turn (`WorkItem.context_items`). `None` ⇒ no
    /// carried context — the common case (turn 0 / a leaf carries its bundle inline in
    /// `config_subset`, a non-react Mote has none). Consumed on the next `run` of
    /// `mote_id` and prepended ahead of the F-7 trajectory, so the served model stays
    /// grounded across the whole chain. Default no-op: an executor that doesn't
    /// assemble context ignores it (byte-identical to pre-PR-9d).
    fn set_context_items(&self, _mote_id: MoteId, _context_items_ref: Option<ContentRef>) {}
}
