//! The `StreamEvents` event source: reads a run's journal deltas in `(since_seq,
//! head]`, chunks them into bounded [`EventFrame`](proto::EventFrame)s, and never
//! advances `next_seq` past the journal head. The cursor protocol (`since_seq ->
//! next_seq`, resumable, bounded) is frozen at D120.
//!
//! Split into reusable pieces so both the default snapshot path and the live
//! tailer (R5, in the `kx-gateway` binary) share ONE event source:
//! - [`check_run_ownership`] — the one-time fold-to-head ownership gate.
//! - [`frames_for_range`] — frames for the deltas in `(since_seq, head]`, taking
//!   the already-polled `head` so a live tailer calls it once per advance.
//! - `build_frames` — the snapshot composition (ownership + one range to head),
//!   backing the default [`crate::SnapshotTailer`].
//!
//! Batch C adds the GLOBAL cross-run twin (`StreamAllEvents`): [`GlobalCursor`] /
//! [`seed_global_cursor`] / [`global_frames_for_range`] / `build_global_frames`
//! — same cursor contract, no ownership gate (operator-global; cloud must
//! party-scope or deny), watermark-stamped run attribution per delta.

use kx_journal::JournalEntry;
use kx_projection::RunMetadataFold;
use kx_proto::proto;

use crate::error::{internal, GatewayError};
use crate::reader::JournalReader;

/// Max deltas per frame — bounds frame size (mirrors the coordinator's
/// `READ_ENTRIES_MAX`). A range with more deltas than this is split across frames;
/// the client resumes from each frame's `next_seq`.
const MAX_FRAME_DELTAS: usize = 4096;

/// Validate run ownership (uniform `NotAuthorized`): the run must appear among the
/// journal's `RunRegistered` entries. Folds to head because a registration is typically
/// at seq=1, long before any `since_seq`.
///
/// The live tailer calls this ONCE at subscribe; per-poll re-reads skip it
/// (ownership of an already-registered run cannot change). `O(journal)` once.
///
/// ## Why this reads EVERY registration, not the latest one
///
/// It used to fold the projection and compare against `run_registration()` — a single
/// **last-write-wins slot** (`kx-projection/src/state.rs`), so only the most recently
/// registered run in the whole journal was ever authorized. On a serve, one journal is
/// shared by every submission, and the scaffold calls `register_run` once **per file**.
/// The moment the server began writing file N+1, the browser's in-flight token
/// subscription for file N was refused `permission_denied` and closed — mid-word, with no
/// error surfaced. Any concurrent chat turn or cron fire did the same to any open stream.
/// A registration is a monotone fact, so the correct predicate is membership, not
/// recency: a run that was legitimately registered does not stop being yours because
/// another run started.
///
/// This is a READ gate over already-committed facts. `RunRegistered`'s encoding, the
/// truth fold, and every journaled fact are untouched, so the run digest cannot move.
/// [`RunMetadataFold`] is reused rather than re-deriving "registered" here, so the two
/// cannot drift; folding it is also strictly *cheaper* than the full projection fold it
/// replaces.
pub fn check_run_ownership(
    reader: &dyn JournalReader,
    instance_id: [u8; 16],
) -> Result<(), GatewayError> {
    let head = reader.current_seq().map_err(internal)?;
    // `fold_run_metadata` takes `&dyn Journal`; the gate holds `&dyn JournalReader`. Fold
    // inline with the same accumulator rather than widening that seam for one caller.
    let mut fold = RunMetadataFold::new();
    for entry in reader
        .read_entries_by_seq(1..head.saturating_add(1))
        .map_err(internal)?
    {
        fold.apply(&entry);
    }
    if fold.finish().instance_ids.contains(&instance_id) {
        Ok(())
    } else {
        Err(GatewayError::NotAuthorized)
    }
}

/// Build resumable frames for the surfaced deltas in `(since_seq, head]`, the
/// caller supplying the already-polled `head`. Assumes ownership was already
/// checked (the snapshot path checks in `build_frames`; the live tailer checks
/// once at subscribe). `next_seq` is never `> head` by construction; the final
/// frame flags `journal_boundary` at the supplied head.
pub fn frames_for_range(
    reader: &dyn JournalReader,
    since_seq: u64,
    head: u64,
) -> Result<Vec<proto::EventFrame>, GatewayError> {
    // Collect surfaced deltas in (since_seq, head]. The range is half-open
    // `[start, end)`, so `+1` on both bounds yields the inclusive `(since_seq, head]`.
    let mut deltas: Vec<(u64, proto::event_delta::Kind)> = Vec::new();
    let entries = reader
        .read_entries_by_seq(since_seq.saturating_add(1)..head.saturating_add(1))
        .map_err(internal)?;
    for entry in entries {
        if let Some(kind) = entry_to_delta(&entry) {
            deltas.push((entry.seq(), kind));
        }
    }

    let mut frames: Vec<proto::EventFrame> = Vec::new();
    if deltas.is_empty() {
        // Caught up (no surfaced deltas in range): one empty boundary frame so
        // the client advances its cursor to the head and stops re-polling.
        frames.push(proto::EventFrame {
            seq: head,
            deltas: Vec::new(),
            next_seq: head,
            journal_boundary: true,
        });
    } else {
        for chunk in deltas.chunks(MAX_FRAME_DELTAS) {
            let last_seq = chunk.last().map_or(since_seq, |(s, _)| *s);
            let frame_deltas = chunk
                .iter()
                .map(|(seq, kind)| proto::EventDelta {
                    seq: *seq,
                    kind: Some(kind.clone()),
                })
                .collect();
            frames.push(proto::EventFrame {
                seq: last_seq,
                deltas: frame_deltas,
                next_seq: last_seq,
                journal_boundary: false,
            });
        }
        // After reading the whole range, the client is caught up to head: the
        // final frame advances to head and flags the boundary. next_seq is never
        // > head by construction.
        if let Some(last) = frames.last_mut() {
            last.next_seq = head;
            last.journal_boundary = true;
        }
    }
    Ok(frames)
}

/// The snapshot composition for `StreamEvents(instance_id, since_seq)`: validate
/// ownership, then emit the frames for `(since_seq, head]` once. Backs the default
/// [`crate::SnapshotTailer`]; the live tailer composes the pieces itself so it can
/// re-poll the head.
pub(crate) fn build_frames(
    reader: &dyn JournalReader,
    instance_id: [u8; 16],
    since_seq: u64,
) -> Result<Vec<proto::EventFrame>, GatewayError> {
    check_run_ownership(reader, instance_id)?;
    let head = reader.current_seq().map_err(internal)?;
    frames_for_range(reader, since_seq, head)
}

// ---------------------------------------------------------------------------
// Batch C — the GLOBAL cross-run tail (`StreamAllEvents`). Same chunk/boundary
// cursor contract as the per-run pieces above; two deliberate differences:
// NO ownership gate (operator-global — the host auth interceptor is the gate;
// CLOUD must party-scope or deny, the proto flag), and a STATEFUL cursor that
// carries the run-attribution WATERMARK (the latest `RunRegistered` at-or-below
// the current seq — the capture.db `run_meta` precedent) so every delta is
// stamped with the run it belongs to. Attribution is display/observability,
// never identity.
// ---------------------------------------------------------------------------

/// The global tail's resumable cursor: the seq watermark plus the run
/// attribution in force at that seq (the latest `RunRegistered` at-or-below).
/// Seeded once at subscribe by [`seed_global_cursor`]; advanced per emitted
/// range by [`global_frames_for_range`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalCursor {
    /// The highest seq already emitted (the resume point).
    pub seq: u64,
    /// The latest `RunRegistered` instance at-or-below `seq` (`None` before any
    /// registration — deltas stamp EMPTY, the honest pre-registration answer).
    pub instance: Option<[u8; 16]>,
}

/// Seed the global cursor for a resume from `since_seq`: one pass over
/// `[0, since_seq]` capturing the latest `RunRegistered` so the first emitted
/// delta is attributed correctly even when the registration folded before the
/// resume point. `O(journal-prefix)` once per subscribe (the same cost class as
/// the per-run subscribe's ownership fold).
///
/// # Errors
/// `Internal` on a journal read failure.
pub fn seed_global_cursor(
    reader: &dyn JournalReader,
    since_seq: u64,
) -> Result<GlobalCursor, GatewayError> {
    let mut instance: Option<[u8; 16]> = None;
    let entries = reader
        .read_entries_by_seq(0..since_seq.saturating_add(1))
        .map_err(internal)?;
    for entry in entries {
        if let JournalEntry::RunRegistered { instance_id, .. } = entry {
            instance = Some(instance_id);
        }
    }
    Ok(GlobalCursor {
        seq: since_seq,
        instance,
    })
}

/// Build resumable GLOBAL frames for the surfaced deltas in `(cursor.seq, head]`,
/// stamping each with the watermark attribution and advancing the cursor (seq +
/// watermark) as `RunRegistered` entries fold past. Same chunking/boundary
/// contract as [`frames_for_range`]; `next_seq` is never `> head`.
///
/// # Errors
/// `Internal` on a journal read failure.
pub fn global_frames_for_range(
    reader: &dyn JournalReader,
    cursor: &mut GlobalCursor,
    head: u64,
) -> Result<Vec<proto::GlobalEventFrame>, GatewayError> {
    let since_seq = cursor.seq;
    let mut deltas: Vec<proto::GlobalEventDelta> = Vec::new();
    let entries = reader
        .read_entries_by_seq(since_seq.saturating_add(1)..head.saturating_add(1))
        .map_err(internal)?;
    for entry in entries {
        // Advance the watermark BEFORE mapping, so the RunRegistered delta itself
        // (and everything after it) is stamped with the NEW run.
        if let JournalEntry::RunRegistered { instance_id, .. } = &entry {
            cursor.instance = Some(*instance_id);
        }
        if let Some(kind) = entry_to_global_delta(&entry) {
            deltas.push(proto::GlobalEventDelta {
                seq: entry.seq(),
                instance_id: cursor.instance.map(|i| i.to_vec()).unwrap_or_default(),
                kind: Some(kind),
            });
        }
    }

    let mut frames: Vec<proto::GlobalEventFrame> = Vec::new();
    if deltas.is_empty() {
        frames.push(proto::GlobalEventFrame {
            seq: head,
            deltas: Vec::new(),
            next_seq: head,
            journal_boundary: true,
        });
    } else {
        let chunks: Vec<Vec<proto::GlobalEventDelta>> = deltas
            .chunks(MAX_FRAME_DELTAS)
            .map(<[proto::GlobalEventDelta]>::to_vec)
            .collect();
        for chunk in chunks {
            let last_seq = chunk.last().map_or(since_seq, |d| d.seq);
            frames.push(proto::GlobalEventFrame {
                seq: last_seq,
                deltas: chunk,
                next_seq: last_seq,
                journal_boundary: false,
            });
        }
        if let Some(last) = frames.last_mut() {
            last.next_seq = head;
            last.journal_boundary = true;
        }
    }
    cursor.seq = head;
    Ok(frames)
}

/// The snapshot composition for `StreamAllEvents(since_seq)`: seed the cursor,
/// then emit the frames for `(since_seq, head]` once. Backs the default
/// [`crate::SnapshotGlobalTailer`]; the live tailer composes the pieces itself
/// so it can re-poll the head. NO ownership gate — operator-global by design
/// (the host auth interceptor gates the RPC; the proto flags the cloud rule).
pub(crate) fn build_global_frames(
    reader: &dyn JournalReader,
    since_seq: u64,
) -> Result<Vec<proto::GlobalEventFrame>, GatewayError> {
    let mut cursor = seed_global_cursor(reader, since_seq)?;
    let head = reader.current_seq().map_err(internal)?;
    global_frames_for_range(reader, &mut cursor, head)
}

/// Map a journal entry to a GLOBAL streamed delta kind: everything the per-run
/// cursor surfaces, PLUS `RunRegistered` (the run-start fact the global feed
/// narrates; the frozen per-run cursor never surfaces it).
fn entry_to_global_delta(entry: &JournalEntry) -> Option<proto::global_event_delta::Kind> {
    use proto::global_event_delta::Kind;
    if let JournalEntry::RunRegistered {
        recipe_fingerprint,
        ts,
        ..
    } = entry
    {
        return Some(Kind::RunRegistered(proto::RunRegisteredDelta {
            recipe_fingerprint: recipe_fingerprint.to_vec(),
            registered_unix_ms: *ts,
        }));
    }
    entry_to_delta(entry).map(|kind| match kind {
        proto::event_delta::Kind::Committed(c) => Kind::Committed(c),
        proto::event_delta::Kind::Failed(f) => Kind::Failed(f),
        proto::event_delta::Kind::Repudiated(r) => Kind::Repudiated(r),
        proto::event_delta::Kind::EffectStaged(e) => Kind::EffectStaged(e),
    })
}

/// Map a journal entry to a streamed delta, or `None` for kinds the cursor does
/// not surface (Proposed / RunRegistered / RunVersionsResolved / DigestSealed).
fn entry_to_delta(entry: &JournalEntry) -> Option<proto::event_delta::Kind> {
    match entry {
        JournalEntry::Committed {
            mote_id,
            result_ref,
            nondeterminism,
            ..
        } => Some(proto::event_delta::Kind::Committed(proto::CommittedDelta {
            mote_id: mote_id.as_bytes().to_vec(),
            result_ref: result_ref.0.to_vec(),
            nd_class: proto::NdClass::from(*nondeterminism) as i32,
        })),
        JournalEntry::Failed {
            mote_id,
            reason_class,
            ..
        } => Some(proto::event_delta::Kind::Failed(proto::FailedDelta {
            mote_id: mote_id.as_bytes().to_vec(),
            reason_class: *reason_class as u32,
        })),
        JournalEntry::Repudiated {
            target_mote_id,
            target_committed_seq,
            ..
        } => Some(proto::event_delta::Kind::Repudiated(
            proto::RepudiatedDelta {
                target_mote_id: target_mote_id.as_bytes().to_vec(),
                target_committed_seq: *target_committed_seq,
            },
        )),
        JournalEntry::EffectStaged { mote_id, .. } => Some(proto::event_delta::Kind::EffectStaged(
            proto::EffectStagedDelta {
                mote_id: mote_id.as_bytes().to_vec(),
            },
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use kx_journal::{InMemoryJournal, Journal, INSTANCE_ID_LEN};
    use kx_mote::{MoteDefHash, MoteId, NdClass};
    use smallvec::SmallVec;

    use crate::reader::ReadOnly;

    use super::*;

    fn reg(instance: u8, recipe: u8, ts: u64) -> JournalEntry {
        JournalEntry::RunRegistered {
            instance_id: [instance; INSTANCE_ID_LEN],
            recipe_fingerprint: [recipe; 32],
            ts,
            seq: 0,
        }
    }

    /// A Committed entry with a distinct identity derived from `n` (the journal
    /// dedups by idempotency key, so each test commit needs its own).
    fn committed(n: u32) -> JournalEntry {
        let mut id = [0u8; 32];
        id[..4].copy_from_slice(&n.to_le_bytes());
        JournalEntry::Committed {
            mote_id: MoteId::from_bytes(id),
            idempotency_key: id,
            seq: 0,
            nondeterminism: NdClass::Pure,
            result_ref: kx_content::ContentRef::from_bytes(id),
            parents: SmallVec::new(),
            warrant_ref: kx_content::ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([0x09; 32]),
        }
    }

    fn instance_of(delta: &proto::GlobalEventDelta) -> Vec<u8> {
        delta.instance_id.clone()
    }

    #[test]
    fn global_frames_surface_run_registered_and_stamp_instance() {
        let j = InMemoryJournal::new();
        j.append(reg(7, 8, 1234)).unwrap(); // seq 1
        j.append(committed(1)).unwrap(); // seq 2
        let r = ReadOnly::new(j);

        let frames = build_global_frames(&r, 0).unwrap();
        assert_eq!(frames.len(), 1);
        let frame = &frames[0];
        assert_eq!(frame.deltas.len(), 2);
        assert!(frame.journal_boundary);
        assert_eq!(frame.next_seq, 2);

        // The registration delta surfaces (the per-run cursor never emits it),
        // stamped with its OWN run.
        let regd = &frame.deltas[0];
        assert_eq!(regd.seq, 1);
        assert_eq!(instance_of(regd), vec![7u8; INSTANCE_ID_LEN]);
        match regd.kind.as_ref().unwrap() {
            proto::global_event_delta::Kind::RunRegistered(rr) => {
                assert_eq!(rr.recipe_fingerprint, vec![8u8; 32]);
                assert_eq!(rr.registered_unix_ms, 1234);
            }
            other => panic!("expected RunRegistered, got {other:?}"),
        }
        // The commit after it carries the same watermark.
        assert_eq!(instance_of(&frame.deltas[1]), vec![7u8; INSTANCE_ID_LEN]);
        assert!(matches!(
            frame.deltas[1].kind,
            Some(proto::global_event_delta::Kind::Committed(_))
        ));
    }

    #[test]
    fn check_run_ownership_gates_on_registration() {
        // The run-ownership gate the live token stream (PR-4.2) reuses: the caller
        // must own `instance_id`; a foreign instance is refused (uniform
        // NotAuthorized — no existence oracle). The streamed `mote_id` is the
        // broker key, NOT a second journal gate (a freshly-submitted terminal mote
        // is not journaled when the client subscribes for time-to-first-token).
        let j = InMemoryJournal::new();
        j.append(reg(7, 8, 1234)).unwrap();
        j.append(committed(1)).unwrap();
        let r = ReadOnly::new(j);

        assert!(check_run_ownership(&r, [7u8; INSTANCE_ID_LEN]).is_ok());
        assert!(matches!(
            check_run_ownership(&r, [9u8; INSTANCE_ID_LEN]),
            Err(GatewayError::NotAuthorized)
        ));
    }

    #[test]
    fn an_earlier_run_stays_authorized_after_a_later_one_registers() {
        // THE REGRESSION THIS GATE SHIPPED WITH. The gate compared against the
        // projection's `run_registration()` — a last-write-wins slot — so registering a
        // second run silently revoked the first. On a serve that is not an edge case: one
        // journal is shared by every submission, and the scaffold registers a run PER
        // FILE, so a token stream following file N was refused the instant file N+1
        // began. The user saw a pane that simply stopped mid-word.
        //
        // A registration is monotone. Both runs are authorized; an unregistered third is
        // still refused, so widening membership did not weaken the gate.
        let j = InMemoryJournal::new();
        j.append(reg(0xa1, 1, 10)).unwrap();
        j.append(committed(1)).unwrap();
        j.append(reg(0xb2, 2, 20)).unwrap();
        j.append(committed(2)).unwrap();
        let r = ReadOnly::new(j);

        assert!(
            check_run_ownership(&r, [0xa1; INSTANCE_ID_LEN]).is_ok(),
            "the EARLIER run must remain authorized once a later run registers"
        );
        assert!(check_run_ownership(&r, [0xb2; INSTANCE_ID_LEN]).is_ok());
        assert!(matches!(
            check_run_ownership(&r, [0xc3; INSTANCE_ID_LEN]),
            Err(GatewayError::NotAuthorized)
        ));
    }

    #[test]
    fn two_runs_attribute_to_the_latest_registration() {
        // The watermark pin: reg A → commit → reg B → commit. The first commit
        // belongs to A, the second to B (latest registration at-or-below its seq).
        let j = InMemoryJournal::new();
        j.append(reg(0xa1, 1, 10)).unwrap(); // seq 1
        j.append(committed(1)).unwrap(); // seq 2 → run A
        j.append(reg(0xb2, 2, 20)).unwrap(); // seq 3
        j.append(committed(2)).unwrap(); // seq 4 → run B
        let r = ReadOnly::new(j);

        let frames = build_global_frames(&r, 0).unwrap();
        let deltas: Vec<_> = frames.iter().flat_map(|f| f.deltas.iter()).collect();
        assert_eq!(deltas.len(), 4);
        assert_eq!(instance_of(deltas[0]), vec![0xa1; INSTANCE_ID_LEN]);
        assert_eq!(instance_of(deltas[1]), vec![0xa1; INSTANCE_ID_LEN]);
        assert_eq!(instance_of(deltas[2]), vec![0xb2; INSTANCE_ID_LEN]);
        assert_eq!(instance_of(deltas[3]), vec![0xb2; INSTANCE_ID_LEN]);
    }

    #[test]
    fn seed_resumes_attribution_past_the_registration() {
        // A resume whose `since_seq` is PAST the registration must still stamp:
        // the seed pass captures the watermark from the journal prefix.
        let j = InMemoryJournal::new();
        j.append(reg(0xc3, 1, 10)).unwrap(); // seq 1
        j.append(committed(1)).unwrap(); // seq 2
        j.append(committed(2)).unwrap(); // seq 3
        let r = ReadOnly::new(j);

        let mut cursor = seed_global_cursor(&r, 2).unwrap();
        assert_eq!(cursor.seq, 2);
        assert_eq!(cursor.instance, Some([0xc3; INSTANCE_ID_LEN]));

        let head = r.current_seq().unwrap();
        let frames = global_frames_for_range(&r, &mut cursor, head).unwrap();
        let deltas: Vec<_> = frames.iter().flat_map(|f| f.deltas.iter()).collect();
        assert_eq!(deltas.len(), 1, "only the seq-3 commit is past the cursor");
        assert_eq!(deltas[0].seq, 3);
        assert_eq!(instance_of(deltas[0]), vec![0xc3; INSTANCE_ID_LEN]);
        assert_eq!(cursor.seq, head, "the cursor advanced to head");
    }

    #[test]
    fn pre_registration_deltas_stamp_empty() {
        // A commit before any registration stamps EMPTY (the honest answer) —
        // never a fabricated id.
        let j = InMemoryJournal::new();
        j.append(committed(1)).unwrap(); // seq 1, no registration yet
        j.append(reg(0xd4, 1, 10)).unwrap(); // seq 2
        let r = ReadOnly::new(j);

        let frames = build_global_frames(&r, 0).unwrap();
        let deltas: Vec<_> = frames.iter().flat_map(|f| f.deltas.iter()).collect();
        assert_eq!(deltas.len(), 2);
        assert!(instance_of(deltas[0]).is_empty());
        assert_eq!(instance_of(deltas[1]), vec![0xd4; INSTANCE_ID_LEN]);
    }

    #[test]
    fn empty_range_emits_one_boundary_frame() {
        let j = InMemoryJournal::new();
        j.append(reg(1, 1, 1)).unwrap();
        let r = ReadOnly::new(j);
        let head = r.current_seq().unwrap();

        // Cursor already at head: one empty boundary frame so the client stops
        // re-polling (the per-run contract, mirrored).
        let mut cursor = seed_global_cursor(&r, head).unwrap();
        let frames = global_frames_for_range(&r, &mut cursor, head).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].deltas.is_empty());
        assert!(frames[0].journal_boundary);
        assert_eq!(frames[0].next_seq, head);
    }

    #[test]
    fn oversize_range_chunks_with_boundary_on_the_last_frame() {
        // > MAX_FRAME_DELTAS surfaced deltas split across frames; only the last
        // flags the boundary and advances next_seq to head (the frozen per-run
        // cursor contract, mirrored on the global twin).
        let j = InMemoryJournal::new();
        j.append(reg(1, 1, 1)).unwrap();
        let n = u32::try_from(MAX_FRAME_DELTAS).unwrap() + 4;
        for i in 0..n {
            j.append(committed(i)).unwrap();
        }
        let r = ReadOnly::new(j);

        let frames = build_global_frames(&r, 0).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].deltas.len(), MAX_FRAME_DELTAS);
        assert!(!frames[0].journal_boundary);
        assert_eq!(
            frames[0].next_seq,
            frames[0].deltas.last().unwrap().seq,
            "a mid-range frame resumes from its own last delta"
        );
        // reg + 4 overflow deltas ride the second frame (the registration is
        // delta 1 of the first chunk... the chunk split is by surfaced count).
        assert!(frames[1].journal_boundary);
        assert_eq!(frames[1].next_seq, r.current_seq().unwrap());
    }
}
