//! F-7 (assemble-into-serve): render a model Mote's resolved Data context — its
//! committed `(parent MoteId, result_ref)` pairs, delivered by the worker's
//! [`kx_worker::ContextSink`] from `WorkItem.parent_results` — into a deterministic
//! text block prepended to the model prompt.
//!
//! **Self-contained + dependency-free** (it deliberately does NOT pull
//! `kx-context-assembler`): the gateway leaf path is a distinct code path from the
//! harness assembler, and its completion is content-addressed by its OWN output, so
//! the only invariant that matters is determinism WITHIN this path — the SAME
//! parents must always render the SAME block, so the leaf's `result_ref` is stable
//! across leases and recovery re-folds (R49). To guarantee that:
//!
//! - parents are **sorted by `MoteId`** (and de-duplicated) before rendering, and
//! - the block is **hard-capped at [`WINDOW_BYTES`]**, failing closed on overflow
//!   (mirroring the assembler's `OverflowDecisionRequired`) so a runaway upstream
//!   can never silently truncate or blow the model window.
//!
//! Empty input ⇒ the empty string ⇒ byte-identical to the pre-F-7 leaf prompt.

use std::fmt;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_mote::MoteId;

/// The hard cap on assembled F-7 context bytes prepended to a model prompt. Pinned
/// (NOT warrant-derived) so the leaf's content-addressed result stays deterministic
/// and a runaway upstream fails closed rather than silently truncating.
pub(crate) const WINDOW_BYTES: usize = 32 * 1024;

/// Failure assembling F-7 serve context. Both variants are fail-closed: the model
/// never runs on partial or unbounded context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AssembleError {
    /// A declared parent `result_ref` did not resolve in the shared store (a forged,
    /// or not-yet-replicated, ref). Never run the model on missing context.
    UpstreamMissing(ContentRef),
    /// The rendered context exceeded [`WINDOW_BYTES`].
    Overflow { needed: usize, cap: usize },
}

impl fmt::Display for AssembleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssembleError::UpstreamMissing(r) => {
                write!(
                    f,
                    "F-7 upstream context {} not resolvable in store",
                    r.to_hex()
                )
            }
            AssembleError::Overflow { needed, cap } => {
                write!(f, "F-7 context {needed}B exceeds window cap {cap}B")
            }
        }
    }
}

/// Render `parents` into a deterministic, labeled context block. Empty input yields
/// the empty string (the pre-F-7 leaf prompt). Fails closed on a missing upstream or
/// a window overflow.
pub(crate) fn assemble_from_parent_results(
    parents: &[(MoteId, ContentRef)],
    store: &LocalFsContentStore,
) -> Result<String, AssembleError> {
    if parents.is_empty() {
        return Ok(String::new());
    }
    // Deterministic order, independent of the wire order the coordinator sent.
    let mut sorted: Vec<(MoteId, ContentRef)> = parents.to_vec();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    sorted.dedup_by(|a, b| a.0 == b.0);

    let mut out = String::new();
    for (_parent_id, result_ref) in &sorted {
        let bytes = store
            .get(result_ref)
            .map_err(|_| AssembleError::UpstreamMissing(*result_ref))?;
        let label = &result_ref.to_hex()[..16];
        // Labeled block; UTF-8-lossy so arbitrary content bytes render deterministically.
        let block = format!(
            "[context parent.{label}]\n{}\n\n",
            String::from_utf8_lossy(bytes.as_ref())
        );
        let needed = out.len() + block.len();
        if needed > WINDOW_BYTES {
            return Err(AssembleError::Overflow {
                needed,
                cap: WINDOW_BYTES,
            });
        }
        out.push_str(&block);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_content::ContentStore;
    use tempfile::TempDir;

    fn store() -> (TempDir, LocalFsContentStore) {
        let dir = TempDir::new().expect("tempdir");
        let store = LocalFsContentStore::open(dir.path()).expect("open store");
        (dir, store)
    }

    fn mote_id(seed: u8) -> MoteId {
        MoteId::from_bytes([seed; 32])
    }

    #[test]
    fn empty_parents_render_nothing() {
        let (_dir, store) = store();
        assert_eq!(
            assemble_from_parent_results(&[], &store).unwrap(),
            String::new(),
            "no Data context ⇒ empty block ⇒ byte-identical to the pre-F-7 leaf prompt"
        );
    }

    #[test]
    fn render_is_deterministic_and_moteid_sorted() {
        // Two parents inserted in BOTH orders must render byte-identically (sorted by
        // MoteId), so the leaf's content-addressed result_ref is stable (R49).
        let (_dir, store) = store();
        let ref_a = store.put(b"alpha-output").unwrap();
        let ref_b = store.put(b"beta-output").unwrap();
        let id_lo = mote_id(0x01);
        let id_hi = mote_id(0x99);

        let forward =
            assemble_from_parent_results(&[(id_lo, ref_a), (id_hi, ref_b)], &store).unwrap();
        let reverse =
            assemble_from_parent_results(&[(id_hi, ref_b), (id_lo, ref_a)], &store).unwrap();
        assert_eq!(
            forward, reverse,
            "wire order must not change the rendered context"
        );
        // The lower MoteId's block comes first.
        let pos_a = forward.find("alpha-output").unwrap();
        let pos_b = forward.find("beta-output").unwrap();
        assert!(pos_a < pos_b, "blocks ordered by ascending MoteId");
        assert!(forward.contains("[context parent."), "blocks are labeled");
    }

    #[test]
    fn duplicate_parents_are_deduped() {
        let (_dir, store) = store();
        let r = store.put(b"once").unwrap();
        let id = mote_id(0x42);
        let out = assemble_from_parent_results(&[(id, r), (id, r)], &store).unwrap();
        assert_eq!(
            out.matches("once").count(),
            1,
            "a repeated parent renders once"
        );
    }

    #[test]
    fn missing_upstream_fails_closed() {
        let (_dir, store) = store();
        // A result_ref that was never put → not resolvable → fail closed (never run the
        // model on missing context).
        let phantom = ContentRef::of(b"never-stored");
        let err = assemble_from_parent_results(&[(mote_id(1), phantom)], &store).unwrap_err();
        assert_eq!(err, AssembleError::UpstreamMissing(phantom));
    }

    #[test]
    fn overflow_fails_closed() {
        let (_dir, store) = store();
        // A single parent larger than the window must fail closed (no silent truncation).
        let big = vec![b'x'; WINDOW_BYTES + 1];
        let r = store.put(&big).unwrap();
        let err = assemble_from_parent_results(&[(mote_id(1), r)], &store).unwrap_err();
        assert!(matches!(err, AssembleError::Overflow { cap, .. } if cap == WINDOW_BYTES));
    }
}
