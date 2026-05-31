//! Native deterministic-critic execution (P4.2-2). A critic Mote
//! (`critic_check = Some(spec)`, `critic_for = Some(producer)`, `Pure`,
//! `!is_topology_shaper` — executor refusal R-15) is run by the runtime
//! **in-process**: it reads its producer's committed output bytes, evaluates the
//! declared [`kx_critic_types::CheckSpec`] via [`kx_critic::evaluate`] (no
//! `execvp`, no model call), and commits the resulting
//! [`kx_critic_types::CriticVerdict`] as its own `result_ref`.
//!
//! The verdict is the content-addressed trust FACT the projection's promotion
//! gate reads (P4.2-3). A clean `Invalid` verdict is a **successful** critic
//! commit, NOT a `Failed` — the gate (not the critic) withholds the producer.
//! Because [`kx_critic::evaluate`] is pure/total/deterministic and the verdict's
//! ref is `blake3(verdict.encode())`, the same producer bytes commit a
//! byte-identical verdict on every run/process/machine (SN-8 exact equality;
//! integer-only evidence, no float on the identity/commit path).

use kx_content::{ContentRef, ContentStore};
use kx_critic_types::CriticVerdict;
use kx_journal::{Journal, JournalEntry};
use kx_mote::{Mote, MoteId, NdClass};
use kx_warrant::WarrantSpec;
use smallvec::SmallVec;

use crate::lifecycle::{LifecycleCommit, LifecycleError};

/// Run a native deterministic-critic Mote to commit.
///
/// Steps:
/// 1. Run-time R-15 guard (defense-in-depth; the submission-time R-15 predicate
///    in `kx_refusal` is the primary enforcement).
/// 2. **P0.4 hard gate** — if this critic is already committed, serve the
///    committed verdict ref, never re-evaluate.
/// 3. Read the producer's committed output bytes (the critic has a Data edge to
///    its producer, so the scheduler only makes it ready AFTER the producer
///    commits).
/// 4. `kx_critic::evaluate(spec, producer_bytes)` → [`CriticVerdict`].
/// 5. `store.put(verdict.encode())` → the verdict's content-addressed ref.
/// 6. Commit `Proposed` + `Committed` (PURE; no broker dispatch).
///
/// # Errors
///
/// [`LifecycleError::Internal`] on an R-15 violation, a missing producer commit
/// (scheduling-invariant violation), or a content-store read/write failure;
/// [`LifecycleError::JournalAppend`] on a journal failure.
pub fn run_native_critic_mote<J, S>(
    mote: &Mote,
    warrant: &WarrantSpec,
    journal: &J,
    store: &S,
) -> Result<LifecycleCommit, LifecycleError>
where
    J: Journal + ?Sized,
    S: ContentStore + ?Sized,
{
    // 1. Run-time R-15 guard.
    let Some(spec) = &mote.def.critic_check else {
        return Err(LifecycleError::Internal(
            "run_native_critic_mote called on a Mote with no critic_check".into(),
        ));
    };
    if mote.nd_class() != NdClass::Pure || mote.def.is_topology_shaper {
        return Err(LifecycleError::Internal(format!(
            "native critic Mote must be Pure and not a topology shaper (R-15); \
             got nd_class={:?} is_topology_shaper={}",
            mote.nd_class(),
            mote.def.is_topology_shaper
        )));
    }
    let Some(producer_id) = mote.def.critic_for else {
        return Err(LifecycleError::Internal(
            "native critic Mote must declare critic_for (R-15)".into(),
        ));
    };

    // 2. P0.4 hard gate.
    if let Some((committed_seq, result_ref)) = read_committed_ref(journal, &mote.id)? {
        tracing::debug!(critic = ?mote.id, committed_seq, "P0.4 gate: critic already committed — serving verdict, not re-evaluating");
        return Ok(LifecycleCommit {
            committed_seq,
            result_ref,
            mote_id: mote.id,
        });
    }

    // 3. Read the producer's committed output bytes.
    let Some((_, producer_ref)) = read_committed_ref(journal, &producer_id)? else {
        return Err(LifecycleError::Internal(
            "native critic dispatched before its producer committed (scheduling invariant)".into(),
        ));
    };
    let producer_bytes = store
        .get(&producer_ref)
        .map_err(|e| LifecycleError::Internal(format!("read producer bytes for critic: {e:?}")))?;

    // 4. Evaluate the declared check IN-PROCESS — pure, total, deterministic.
    let verdict: CriticVerdict = kx_critic::evaluate(spec, &producer_bytes);

    // 5. Stage the verdict FACT; its ref is blake3(verdict.encode()).
    let verdict_bytes = verdict.encode();
    let result_ref = store
        .put(&verdict_bytes)
        .map_err(|e| LifecycleError::Internal(format!("put critic verdict: {e:?}")))?;

    // 6. Commit Proposed + Committed (PURE).
    let warrant_ref = kx_warrant::warrant_ref_of(warrant);
    journal
        .append(JournalEntry::Proposed {
            mote_id: mote.id,
            idempotency_key: *mote.id.as_bytes(),
            seq: 0,
            nondeterminism: NdClass::Pure,
            placement_hint: 0,
            warrant_ref,
        })
        .map_err(|e| LifecycleError::JournalAppend(format!("Proposed (critic): {e:?}")))?;
    let committed = journal
        .append(JournalEntry::Committed {
            mote_id: mote.id,
            idempotency_key: *mote.id.as_bytes(),
            seq: 0,
            nondeterminism: NdClass::Pure,
            result_ref,
            parents: SmallVec::new(),
            warrant_ref,
            mote_def_hash: mote.def.hash(),
        })
        .map_err(|e| LifecycleError::JournalAppend(format!("Committed (critic): {e:?}")))?;

    Ok(LifecycleCommit {
        committed_seq: committed.seq(),
        result_ref,
        mote_id: mote.id,
    })
}

/// Read a Mote's committed `(seq, result_ref)` from the journal, or `None` if it
/// is not committed. Mirrors `lifecycle::serve_if_committed` (kept local so the
/// critic path is self-contained).
fn read_committed_ref<J: Journal + ?Sized>(
    journal: &J,
    mote_id: &MoteId,
) -> Result<Option<(u64, ContentRef)>, LifecycleError> {
    match journal
        .read_committed(mote_id)
        .map_err(|e| LifecycleError::JournalAppend(format!("read Committed: {e:?}")))?
    {
        Some(JournalEntry::Committed {
            seq, result_ref, ..
        }) => Ok(Some((seq, result_ref))),
        _ => Ok(None),
    }
}
