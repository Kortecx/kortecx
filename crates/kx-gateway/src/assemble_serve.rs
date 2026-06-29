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
//! - the block is **hard-capped at [`crate::env_caps::window_bytes`]**, failing closed on overflow
//!   (mirroring the assembler's `OverflowDecisionRequired`) so a runaway upstream
//!   can never silently truncate or blow the model window.
//!
//! Empty input ⇒ the empty string ⇒ byte-identical to the pre-F-7 leaf prompt.

use std::fmt;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_mote::{ContextItemRef, MoteId};

/// RC3 (context-engineering): bytes reserved for the deterministic truncation
/// marker when [`fit_context_blocks`] / [`fit_trajectory_blocks`] trim an
/// over-window bundle/trajectory. The longest marker (the trajectory variant with
/// every count a 20-digit `usize`) is ~127 bytes, so 192 leaves headroom AND
/// guarantees the trimmed output never exceeds the window cap.
const TRUNCATION_MARKER_RESERVE: usize = 192;

/// Failure assembling F-7 serve context. Both variants are fail-closed: the model
/// never runs on partial or unbounded context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AssembleError {
    /// A declared parent `result_ref` did not resolve in the shared store (a forged,
    /// or not-yet-replicated, ref). Never run the model on missing context.
    UpstreamMissing(ContentRef),
    /// The rendered context exceeded the serve window cap (`KX_SERVE_WINDOW_BYTES`).
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
        let cap = crate::env_caps::window_bytes();
        if needed > cap {
            return Err(AssembleError::Overflow { needed, cap });
        }
        out.push_str(&block);
    }
    Ok(out)
}

/// PR-7 / RC3: render a run's attached context-bundle items (decoded from the entry
/// Mote's `config_subset[CONTEXT_ITEMS_KEY]`) into a deterministic, labeled text
/// block, prepended to the model prompt AHEAD of the F-7 parent context. `items`
/// arrive in canonical RELEVANCE order (chat-rag delivers top-k by similarity; an
/// authored bundle preserves author order); each blob is fetched from the shared
/// store (a `PutContent` ref). Empty ⇒ the empty string (byte-identical to pre-PR-7);
/// a missing ref fails closed (never run the model on PARTIAL context).
///
/// RC3 (context-engineering, D33 §5(a)): on a WINDOW overflow this no longer fails
/// closed — it keeps the highest-relevance contiguous PREFIX that fits and appends a
/// deterministic, bounded truncation MARKER (never silent), via [`fit_context_blocks`].
/// A single item larger than the window still fails closed (`Overflow`) — there is no
/// partial-item rendering. The FITS case is byte-identical to pre-RC3. Off-digest:
/// this block is prepended to the EPHEMERAL prompt, never journaled — the canonical
/// demo attaches no bundle ⇒ empty ⇒ `7d22d4bd` untouched. (The F-7 PARENT path
/// [`assemble_from_parent_results`] stays fail-closed: it is `MoteId`-sorted, not
/// relevance-ordered, so a prefix is not a meaningful trim — trajectory bounding is a
/// coordinator concern.) Bytes render UTF-8-lossy — the serve text path.
pub(crate) fn assemble_context_items(
    items: &[ContextItemRef],
    store: &LocalFsContentStore,
) -> Result<String, AssembleError> {
    if items.is_empty() {
        return Ok(String::new());
    }
    // Render every item to its labeled block first (fail closed on a missing ref —
    // never run the model on partial context). Canonical input order = relevance order.
    let mut blocks: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let cref = ContentRef(item.content_ref);
        let bytes = store
            .get(&cref)
            .map_err(|_| AssembleError::UpstreamMissing(cref))?;
        let label = if item.name.is_empty() {
            cref.to_hex()[..16].to_string()
        } else {
            item.name.clone()
        };
        blocks.push(format!(
            "[context {label}]\n{}\n\n",
            String::from_utf8_lossy(bytes.as_ref())
        ));
    }
    let cap = crate::env_caps::window_bytes();
    fit_context_blocks(&blocks, cap).map_err(|needed| AssembleError::Overflow { needed, cap })
}

/// RC3 (context-engineering, D33 §5(a)): fit pre-rendered, relevance-ordered context
/// `blocks` into `cap` bytes. PURE + total + deterministic (no I/O, no env, no clock)
/// so it is unit-testable at an explicit cap and replay-stable.
///
/// - All blocks fit (`Σ ≤ cap`) ⇒ `Ok(blocks.concat())`, **byte-identical** to the
///   untrimmed bundle (the pre-RC3 behavior for the common case).
/// - Otherwise ⇒ keep the highest-relevance contiguous PREFIX that fits within
///   `cap - TRUNCATION_MARKER_RESERVE`, then append a deterministic, bounded marker
///   `"[context truncated: kept k/N items, dropped Bb]"` — NEVER silent truncation.
///   The result is guaranteed `≤ cap` (the reserve bounds the marker).
/// - `Err(needed)` iff even the first (most-relevant) block cannot fit the budget — a
///   single oversized item fails closed upstream (`Overflow`), as before.
fn fit_context_blocks(blocks: &[String], cap: usize) -> Result<String, usize> {
    let total: usize = blocks.iter().map(String::len).sum();
    if total <= cap {
        return Ok(blocks.concat());
    }
    let budget = cap.saturating_sub(TRUNCATION_MARKER_RESERVE);
    let mut running = 0usize;
    let mut kept = 0usize;
    for block in blocks {
        if running + block.len() > budget {
            break;
        }
        running += block.len();
        kept += 1;
    }
    if kept == 0 {
        return Err(blocks.first().map_or(0, String::len));
    }
    let dropped_bytes: usize = blocks[kept..].iter().map(String::len).sum();
    let mut out = String::with_capacity(running + TRUNCATION_MARKER_RESERVE);
    for block in &blocks[..kept] {
        out.push_str(block);
    }
    // Bounded, deterministic honesty marker — NEVER silent truncation. Built into a
    // local first (clippy `format_push_string`-clean), then appended.
    let marker = format!(
        "[context truncated: kept {kept}/{} items, dropped {dropped_bytes}B]\n\n",
        blocks.len()
    );
    out.push_str(&marker);
    Ok(out)
}

/// RC3 (BUG-FIX + context-engineering): render an already-ORDERED ReAct trajectory
/// — the coordinator's `resolve_parent_context` delivers turns 0..T-1 + their tool
/// observations in TURN-ascending (transcript) order (D78: the model must read the
/// conversation in TIME order). Unlike [`assemble_from_parent_results`] this does
/// **NOT** re-sort by `MoteId` — doing so (the pre-RC3 live-serve path) scrambled the
/// conversation, because ReAct turn/observation `MoteId`s are run-salted blake3 hashes,
/// non-monotonic in turn (BUG: the live model read its own steps out of order). Order
/// is preserved verbatim; it is already deterministic (turn numbers are fixed), so the
/// leaf `result_ref` stays stable across leases/recovery (R49).
///
/// On a window overflow this keeps the most-RECENT contiguous SUFFIX that fits (the
/// recency window — recent tool observations matter most to the next step) and prepends
/// a deterministic, bounded marker; a single most-recent item larger than the window
/// fails closed (`Overflow`). Empty ⇒ `""`. Off-digest: prepended to the EPHEMERAL
/// prompt, never journaled.
pub(crate) fn assemble_trajectory(
    entries: &[(MoteId, ContentRef)],
    store: &LocalFsContentStore,
) -> Result<String, AssembleError> {
    if entries.is_empty() {
        return Ok(String::new());
    }
    // Render each entry IN ORDER (fail closed on a missing ref). No MoteId sort, no
    // dedup — the coordinator already ordered (and the trajectory has no duplicates).
    let mut blocks: Vec<String> = Vec::with_capacity(entries.len());
    for (_mote_id, result_ref) in entries {
        let bytes = store
            .get(result_ref)
            .map_err(|_| AssembleError::UpstreamMissing(*result_ref))?;
        let label = &result_ref.to_hex()[..16];
        blocks.push(format!(
            "[context parent.{label}]\n{}\n\n",
            String::from_utf8_lossy(bytes.as_ref())
        ));
    }
    let cap = crate::env_caps::window_bytes();
    fit_trajectory_blocks(&blocks, cap).map_err(|needed| AssembleError::Overflow { needed, cap })
}

/// RC3 (context-engineering): fit time-ordered (oldest-first) trajectory `blocks` into
/// `cap` bytes, keeping the most-RECENT contiguous SUFFIX. PURE + deterministic.
///
/// - All fit (`Σ ≤ cap`) ⇒ `Ok(blocks.concat())`, byte-identical to the untrimmed
///   trajectory (the common case — `max_turns` keeps chains short).
/// - Otherwise ⇒ drop the OLDEST blocks, keep the recent suffix that fits within
///   `cap - TRUNCATION_MARKER_RESERVE`, and PREPEND a bounded marker noting the drop
///   (NEVER silent). The result is guaranteed `≤ cap`.
/// - `Err(needed)` iff even the single most-recent block cannot fit — fail closed.
fn fit_trajectory_blocks(blocks: &[String], cap: usize) -> Result<String, usize> {
    let total: usize = blocks.iter().map(String::len).sum();
    if total <= cap {
        return Ok(blocks.concat());
    }
    let budget = cap.saturating_sub(TRUNCATION_MARKER_RESERVE);
    let mut running = 0usize;
    let mut kept = 0usize;
    // Walk from the END (most recent) backward — the recency window.
    for block in blocks.iter().rev() {
        if running + block.len() > budget {
            break;
        }
        running += block.len();
        kept += 1;
    }
    if kept == 0 {
        return Err(blocks.last().map_or(0, String::len));
    }
    let start = blocks.len() - kept;
    let dropped_bytes: usize = blocks[..start].iter().map(String::len).sum();
    // Marker FIRST (the dropped context preceded the kept recent turns). Built into a
    // local (clippy `format_push_string`-clean), then the recent suffix follows.
    let marker = format!(
        "[context: dropped {start} older items ({dropped_bytes}B), kept {kept} most recent]\n\n"
    );
    let mut out = String::with_capacity(running + marker.len());
    out.push_str(&marker);
    for block in &blocks[start..] {
        out.push_str(block);
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
        // (env unset ⇒ window_bytes() == the default cap.)
        let cap = crate::env_caps::DEFAULT_WINDOW_BYTES;
        let big = vec![b'x'; cap + 1];
        let r = store.put(&big).unwrap();
        let err = assemble_from_parent_results(&[(mote_id(1), r)], &store).unwrap_err();
        assert!(matches!(err, AssembleError::Overflow { cap: c, .. } if c == cap));
    }

    // --- PR-7 context items ------------------------------------------------

    #[test]
    fn context_items_render_labeled_blocks() {
        let (_dir, store) = store();
        let r = store.put(b"the spec text").unwrap();
        let items = vec![ContextItemRef {
            name: "spec".into(),
            content_ref: r.0,
        }];
        let out = assemble_context_items(&items, &store).unwrap();
        assert!(out.contains("[context spec]"), "labeled by name");
        assert!(out.contains("the spec text"), "the blob bytes render");
    }

    #[test]
    fn context_items_empty_is_empty() {
        let (_dir, store) = store();
        assert_eq!(assemble_context_items(&[], &store).unwrap(), String::new());
    }

    #[test]
    fn context_items_missing_ref_fails_closed() {
        let (_dir, store) = store();
        let phantom = ContentRef::of(b"never-stored-context");
        let err = assemble_context_items(
            &[ContextItemRef {
                name: "x".into(),
                content_ref: phantom.0,
            }],
            &store,
        )
        .unwrap_err();
        assert_eq!(err, AssembleError::UpstreamMissing(phantom));
    }

    // --- RC3 (context-engineering, D33 §5(a)): deterministic relevance trim --------

    fn block(label: &str, body: &str) -> String {
        format!("[context {label}]\n{body}\n\n")
    }

    #[test]
    fn context_items_fits_is_byte_identical() {
        // Σ ≤ cap ⇒ the untrimmed concatenation, byte-identical to pre-RC3.
        let blocks = vec![block("a", "alpha"), block("b", "beta")];
        let cap = 10_000;
        assert_eq!(
            fit_context_blocks(&blocks, cap).unwrap(),
            blocks.concat(),
            "a bundle that fits is byte-identical to the untrimmed concat"
        );
    }

    #[test]
    fn context_items_overflow_trims_lowest_ranked_suffix_with_marker() {
        // Three relevance-ordered items; a cap that admits the first two + the marker
        // budget but not the third ⇒ keep the prefix, drop the suffix, mark it.
        let blocks = vec![
            block("first", &"A".repeat(200)),
            block("second", &"B".repeat(200)),
            block("third", &"C".repeat(200)),
        ];
        // each block ≈ 200 + label/newlines (~30) ≈ 230B. Cap that fits two blocks
        // (~460B) + the 128B marker reserve but not the third.
        let cap = 460 + TRUNCATION_MARKER_RESERVE;
        let out = fit_context_blocks(&blocks, cap).unwrap();
        assert!(
            out.contains("[context first]"),
            "kept the most relevant: {out}"
        );
        assert!(out.contains("[context second]"), "kept the 2nd: {out}");
        assert!(
            !out.contains("[context third]"),
            "dropped the suffix: {out}"
        );
        assert!(
            out.contains("[context truncated: kept 2/3 items, dropped"),
            "honesty marker present: {out}"
        );
        assert!(
            out.len() <= cap,
            "trimmed output stays within the window cap"
        );
    }

    #[test]
    fn context_items_single_oversized_item_still_fails_closed() {
        // One item larger than the window ⇒ Err(needed) ⇒ Overflow upstream (no
        // partial-item rendering — same fail-closed guarantee as the parent path).
        let blocks = vec![block("huge", &"X".repeat(1000))];
        let cap = 100;
        let err = fit_context_blocks(&blocks, cap).unwrap_err();
        assert_eq!(
            err,
            blocks[0].len(),
            "needed = the oversized block's length"
        );
    }

    #[test]
    fn context_items_trim_is_deterministic() {
        let blocks = vec![
            block("one", &"1".repeat(300)),
            block("two", &"2".repeat(300)),
            block("three", &"3".repeat(300)),
        ];
        let cap = 350 + TRUNCATION_MARKER_RESERVE;
        let a = fit_context_blocks(&blocks, cap).unwrap();
        let b = fit_context_blocks(&blocks, cap).unwrap();
        assert_eq!(a, b, "same blocks + cap ⇒ byte-identical trim");
    }

    // --- RC3 (BUG-FIX): ReAct trajectory renders in TIME order, not MoteId order -----

    #[test]
    fn trajectory_preserves_input_time_order_not_moteid() {
        // Regression pin for the live-serve react ordering bug: the trajectory must
        // render in the coordinator's TIME order (input order), NOT re-sorted by
        // MoteId. Entry 0 (first in time) has a HIGH MoteId; entry 1 has a LOW MoteId,
        // so a MoteId sort would FLIP them.
        let (_dir, store) = store();
        let ref_first = store.put(b"turn0-first-in-time").unwrap();
        let ref_second = store.put(b"turn1-second-in-time").unwrap();
        let id_hi = mote_id(0x99); // first in time, but high MoteId
        let id_lo = mote_id(0x01); // second in time, but low MoteId
        let entries = [(id_hi, ref_first), (id_lo, ref_second)];

        let out = assemble_trajectory(&entries, &store).unwrap();
        let pos_first = out.find("turn0-first-in-time").unwrap();
        let pos_second = out.find("turn1-second-in-time").unwrap();
        assert!(
            pos_first < pos_second,
            "trajectory must preserve TIME order regardless of MoteId: {out}"
        );

        // Contrast: the MoteId-sorted parent path WOULD flip them (documents the bug
        // the fix avoids — the react path must NOT use this renderer).
        let moteid_sorted = assemble_from_parent_results(&entries, &store).unwrap();
        assert!(
            moteid_sorted.find("turn1-second-in-time").unwrap()
                < moteid_sorted.find("turn0-first-in-time").unwrap(),
            "the MoteId-sorted path flips time order (the bug the trajectory path avoids)"
        );
    }

    #[test]
    fn trajectory_empty_is_empty() {
        let (_dir, store) = store();
        assert_eq!(assemble_trajectory(&[], &store).unwrap(), String::new());
    }

    #[test]
    fn trajectory_missing_ref_fails_closed() {
        let (_dir, store) = store();
        let phantom = ContentRef::of(b"never-stored-traj");
        let err = assemble_trajectory(&[(mote_id(1), phantom)], &store).unwrap_err();
        assert_eq!(err, AssembleError::UpstreamMissing(phantom));
    }

    #[test]
    fn trajectory_overflow_keeps_recent_suffix_with_marker() {
        // Oldest-first blocks; a cap admitting the two MOST-RECENT + the marker reserve
        // but not the oldest ⇒ drop the oldest, keep the recent suffix, prepend a marker.
        let blocks = vec![
            block("oldest", &"O".repeat(200)),
            block("middle", &"M".repeat(200)),
            block("newest", &"N".repeat(200)),
        ];
        let cap = 460 + TRUNCATION_MARKER_RESERVE;
        let out = fit_trajectory_blocks(&blocks, cap).unwrap();
        assert!(
            !out.contains("[context oldest]"),
            "dropped the oldest: {out}"
        );
        assert!(out.contains("[context middle]"), "kept the recent: {out}");
        assert!(
            out.contains("[context newest]"),
            "kept the most recent: {out}"
        );
        assert!(
            out.contains("[context: dropped 1 older items"),
            "honesty marker present: {out}"
        );
        // Recency: the kept blocks stay in time order (middle before newest).
        assert!(out.find("[context middle]").unwrap() < out.find("[context newest]").unwrap());
        assert!(
            out.len() <= cap,
            "trimmed output stays within the window cap"
        );
    }

    #[test]
    fn trajectory_single_oversized_item_still_fails_closed() {
        let blocks = vec![block("huge", &"X".repeat(1000))];
        let err = fit_trajectory_blocks(&blocks, 100).unwrap_err();
        assert_eq!(err, blocks[0].len());
    }

    #[test]
    fn trajectory_fits_is_byte_identical_and_deterministic() {
        let blocks = vec![block("a", "alpha"), block("b", "beta")];
        let cap = 10_000;
        let a = fit_trajectory_blocks(&blocks, cap).unwrap();
        assert_eq!(
            a,
            blocks.concat(),
            "a trajectory that fits is the untrimmed concat"
        );
        assert_eq!(
            a,
            fit_trajectory_blocks(&blocks, cap).unwrap(),
            "deterministic"
        );
    }
}
