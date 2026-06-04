//! The `StreamEvents` cursor. Reads the run's journal deltas in `(since_seq,
//! head]`, chunks them into bounded [`EventFrame`](proto::EventFrame)s, and never
//! advances `next_seq` past the journal head. For the D120 freeze this is a
//! snapshot-to-head stream; the cursor protocol (`since_seq -> next_seq`,
//! resumable, bounded) is frozen now so live tailing (M8) is additive.

use kx_journal::JournalEntry;
use kx_proto::proto;

use crate::error::{internal, GatewayError};
use crate::reader::JournalReader;
use crate::view::fold_through;

/// Max deltas per frame — bounds frame size (mirrors the coordinator's
/// `READ_ENTRIES_MAX`). A run with more deltas than this is split across frames;
/// the client resumes from each frame's `next_seq`.
const MAX_FRAME_DELTAS: usize = 4096;

/// Build the frames for `StreamEvents(instance_id, since_seq)`. Validates run
/// ownership first (uniform `NotAuthorized`), then emits resumable frames.
pub(crate) fn build_frames(
    reader: &dyn JournalReader,
    instance_id: [u8; 16],
    since_seq: u64,
) -> Result<Vec<proto::EventFrame>, GatewayError> {
    let head = reader.current_seq().map_err(internal)?;

    // Ownership: fold to head to read the run's registration (RunRegistered at
    // seq=1 may be before `since_seq`).
    let (projection, _) = fold_through(reader, head)?;
    match projection.run_registration() {
        Some((inst, _)) if inst == instance_id => {}
        _ => return Err(GatewayError::NotAuthorized),
    }

    // Collect surfaced deltas in (since_seq, head].
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
