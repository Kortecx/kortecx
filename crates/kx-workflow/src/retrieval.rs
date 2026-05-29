//! Graph-RAG / vector retrieval as a **`ReadOnlyNondet` Mote** — the SN-8-safe
//! way to bring similarity search into a Morphic workflow.
//!
//! A retrieval step reads a [`kx_dataset::RetrievalIndex`] (a nondeterministic,
//! similarity-based read of the world) and commits its result as a
//! **content-addressed fact**; everything downstream consumes that fact by
//! **exact** hash. The similarity lives entirely inside the Mote body — it is
//! NEVER an operator on the identity / commit / memoization path (the runtime
//! matches by exact cryptographic equality only, SN-8).
//!
//! The committed fact is the ORDERED set of retrieved content refs — **scores
//! are deliberately excluded** ([`encode_retrieval_fact`]). Two index states
//! that return the same neighbours produce the same fact regardless of the
//! float similarity scores, so similarity cannot leak into a `MoteId`.

use kx_content::ContentRef;
use kx_dataset::Hit;
use kx_mote::{EffectPattern, LogicRef, ModelId, NdClass, ToolName};
use kx_warrant::WarrantSpec;

use crate::def::{StepDef, StepRole};
use crate::synthesis::step;

/// A retrieval step: a graph-RAG / vector lookup modelled as a `ReadOnlyNondet`
/// Mote whose committed `result_ref` is the retrieved set. ROND because which
/// neighbours come back depends on the (mutable) index state — a nondet read;
/// `StageThenCommit` because the retrieved set is staged then committed as the
/// Mote's fact. The similarity search itself is the Mote's runtime logic
/// (`logic_ref`), confined behind the SN-8 boundary.
#[must_use]
pub fn retrieval(
    logic_ref: LogicRef,
    model_id: ModelId,
    warrant: WarrantSpec,
    capability: ToolName,
) -> StepDef {
    step(
        logic_ref,
        model_id,
        NdClass::ReadOnlyNondet,
        EffectPattern::StageThenCommit,
        StepRole::Plain,
        warrant,
        capability,
    )
}

/// Canonical bytes of a retrieval RESULT — the ordered content refs only.
///
/// **Scores are excluded by design.** Similarity stays inside the retrieval
/// Mote; the committed fact is the neighbour *set*, matched downstream by exact
/// hash (SN-8). This is the one place the "similarity in, exact fact out"
/// boundary is enforced in code.
#[must_use]
pub fn encode_retrieval_fact(hits: &[Hit]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hits.len().saturating_mul(32));
    for hit in hits {
        out.extend_from_slice(hit.id.as_bytes());
    }
    out
}

/// The content-addressed identity of a retrieval result — what downstream Motes
/// consume by exact hash. Pure over the retrieved ref set (scores excluded).
#[must_use]
pub fn retrieval_result_ref(hits: &[Hit]) -> ContentRef {
    ContentRef::of(&encode_retrieval_fact(hits))
}
