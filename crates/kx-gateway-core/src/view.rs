//! Map a folded [`Projection`] into the server-derived
//! [`ProjectionView`](proto::ProjectionView). Every `MoteSnapshot` field comes
//! from the read API of the fold — the client never computes a `MoteId` (SN-8).
//! Also hosts the read-only fold helper and the `GetContent` authorization.

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_journal::JournalEntry;
use kx_mote::MoteId;
use kx_projection::{AnomalyKind, MoteState, Projection, PromotionState};
use kx_proto::proto;

use crate::error::{internal, GatewayError};
use crate::reader::{ContentReader, JournalReader};
use crate::uploads::UploadsLedger;

/// Fold the run's journal up to and including `at_seq` through the read-only
/// seam (never exposing a writer), returning the [`Projection`] plus a side-map
/// of each committed Mote's `mote_def_hash` (not on the projection read API).
pub(crate) fn fold_through(
    reader: &dyn JournalReader,
    at_seq: u64,
) -> Result<(Projection, BTreeMap<MoteId, [u8; 32]>), GatewayError> {
    let mut projection = Projection::new();
    let mut def_hashes = BTreeMap::new();
    let entries = reader
        .read_entries_by_seq(0..at_seq.saturating_add(1))
        .map_err(internal)?;
    for entry in entries {
        if let JournalEntry::Committed {
            mote_id,
            mote_def_hash,
            ..
        } = &entry
        {
            def_hashes.insert(*mote_id, *mote_def_hash.as_bytes());
        }
        projection.fold(&entry).map_err(internal)?;
    }
    Ok((projection, def_hashes))
}

/// Confirm the folded run belongs to `instance_id`; uniform `NotAuthorized`
/// otherwise (no oracle — same error for wrong-run and unregistered).
fn check_ownership(
    projection: &Projection,
    instance_id: [u8; 16],
) -> Result<[u8; 32], GatewayError> {
    match projection.run_registration() {
        Some((inst, recipe_fp)) if inst == instance_id => Ok(recipe_fp),
        _ => Err(GatewayError::NotAuthorized),
    }
}

/// Build the render-a-run-as-a-DAG view. `at_seq` is clamped to the current head
/// (a future seq yields the head, never an error that leaks the head position).
pub(crate) fn build_view(
    reader: &dyn JournalReader,
    instance_id: [u8; 16],
    at_seq: Option<u64>,
) -> Result<proto::ProjectionView, GatewayError> {
    let head = reader.current_seq().map_err(internal)?;
    let frontier = at_seq.map_or(head, |s| s.min(head));
    let (projection, def_hashes) = fold_through(reader, frontier)?;
    let recipe_fp = check_ownership(&projection, instance_id)?;

    // Compute anomalies ONCE (avoids O(n^2) over the mote set).
    let anomalies: BTreeMap<MoteId, AnomalyKind> = projection.anomaly_motes().into_iter().collect();

    let motes = projection
        .iter_motes()
        .map(|(id, state)| mote_snapshot(&projection, &def_hashes, &anomalies, id, state))
        .collect();

    Ok(proto::ProjectionView {
        instance_id: instance_id.to_vec(),
        recipe_fingerprint: recipe_fp.to_vec(),
        current_seq: frontier,
        motes,
    })
}

/// The run-scope authorized set: every committed (non-repudiated) result ref of
/// the run owned by `instance_id`, from ONE fold. Any ownership failure yields
/// the uniform `NotAuthorized` (no existence oracle).
pub(crate) fn run_authorized_refs(
    reader: &dyn JournalReader,
    instance_id: [u8; 16],
) -> Result<BTreeSet<[u8; 32]>, GatewayError> {
    let head = reader.current_seq().map_err(internal)?;
    let (projection, _) = fold_through(reader, head)?;
    check_ownership(&projection, instance_id)?;
    Ok(projection
        .iter_motes()
        .filter(|(_, state)| *state == MoteState::Committed)
        .filter_map(|(id, _)| projection.result_ref_of(&id))
        .map(|r| r.0)
        .collect())
}

/// `GetContent`: return a committed result by ref, but ONLY if `content_ref` is
/// a committed (non-repudiated) result of the run owned by `instance_id`. Any
/// failure of ownership or the authorized-set check yields the uniform
/// `NotAuthorized` (no existence oracle) — the store is touched only after.
pub(crate) fn get_owned_content(
    reader: &dyn JournalReader,
    content: &dyn ContentReader,
    instance_id: [u8; 16],
    content_ref: [u8; 32],
) -> Result<Vec<u8>, GatewayError> {
    if !run_authorized_refs(reader, instance_id)?.contains(&content_ref) {
        return Err(GatewayError::NotAuthorized);
    }

    // Reachable only by the legitimate owner of a committed ref — a store miss
    // here is a real internal inconsistency, not an oracle.
    content
        .get(&ContentRef::from_bytes(content_ref))
        .ok_or_else(|| internal("committed result missing from the content store"))
}

/// `GetContent`, uploads scope (Batch A — the EMPTY `instance_id`): return an
/// uploaded blob by ref, but ONLY if the uploads ledger recorded it. An absent
/// ledger authorizes nothing, and an unknown ref denies — both as the SAME
/// uniform `NotAuthorized` (no oracle about refs OR about sidecar wiring).
pub(crate) fn get_uploaded_content(
    content: &dyn ContentReader,
    uploads: Option<&dyn UploadsLedger>,
    content_ref: [u8; 32],
) -> Result<Vec<u8>, GatewayError> {
    let ledger = uploads.ok_or(GatewayError::NotAuthorized)?;
    if !ledger.contains(&content_ref)? {
        return Err(GatewayError::NotAuthorized);
    }
    // Reachable only for a recorded upload — the ledger only records refs the
    // store accepted, so a miss is a real internal inconsistency, not an oracle.
    content
        .get(&ContentRef::from_bytes(content_ref))
        .ok_or_else(|| internal("uploaded blob missing from the content store"))
}

/// `GetContentBatch`: resolve up to the handler-capped ref list against ONE
/// authorized set (run scope folds the journal ONCE — the N+1 collapse), in
/// request order. Per-item failures are UNIFORM: an unauthorized, missing, or
/// malformed (non-32-byte) ref yields an empty `payload` + `full_size == 0`,
/// indistinguishable from one another (D120.1). Payloads are truncated at
/// `item_clamp` with `truncated` set and `full_size` honest.
///
/// A bad run TICKET (`instance_id` not owning the run) fails the whole call
/// with the uniform `NotAuthorized` — the same contract as every other
/// run-scoped RPC. `instance_id == None` selects the uploads scope; an absent
/// uploads ledger simply authorizes nothing (all items uniformly empty).
pub(crate) fn get_content_batch(
    reader: &dyn JournalReader,
    content: &dyn ContentReader,
    uploads: Option<&dyn UploadsLedger>,
    instance_id: Option<[u8; 16]>,
    refs: &[Vec<u8>],
    item_clamp: u64,
) -> Result<Vec<proto::ContentBatchItem>, GatewayError> {
    // Run scope: ONE fold for the whole batch. Uploads scope: per-ref ledger
    // membership (point lookups).
    let run_set = match instance_id {
        Some(id) => Some(run_authorized_refs(reader, id)?),
        None => None,
    };

    let empty_item = |raw: &[u8]| proto::ContentBatchItem {
        content_ref: raw.to_vec(),
        payload: Vec::new(),
        truncated: false,
        full_size: 0,
    };

    let mut items = Vec::with_capacity(refs.len());
    for raw in refs {
        let Ok(r) = <[u8; 32]>::try_from(raw.as_slice()) else {
            items.push(empty_item(raw));
            continue;
        };
        let authorized = match &run_set {
            Some(set) => set.contains(&r),
            None => match uploads {
                Some(ledger) => ledger.contains(&r)?,
                None => false,
            },
        };
        if !authorized {
            items.push(empty_item(raw));
            continue;
        }
        // A store miss for an authorized ref degrades to the SAME uniform empty
        // item (batch semantics: per-item, total, no oracle).
        let Some(mut payload) = content.get(&ContentRef::from_bytes(r)) else {
            items.push(empty_item(raw));
            continue;
        };
        let full_size = payload.len() as u64;
        let truncated = full_size > item_clamp;
        if truncated {
            payload.truncate(usize::try_from(item_clamp).unwrap_or(usize::MAX));
        }
        items.push(proto::ContentBatchItem {
            content_ref: raw.clone(),
            payload,
            truncated,
            full_size,
        });
    }
    Ok(items)
}

fn mote_snapshot(
    projection: &Projection,
    def_hashes: &BTreeMap<MoteId, [u8; 32]>,
    anomalies: &BTreeMap<MoteId, AnomalyKind>,
    id: MoteId,
    state: MoteState,
) -> proto::MoteSnapshot {
    let parents = projection
        .parents_of(&id)
        .into_iter()
        .map(|(parent_id, edge)| proto::ParentRef {
            parent_id: parent_id.as_bytes().to_vec(),
            edge_kind: proto::EdgeKind::from(edge.kind) as i32,
            non_cascade: edge.non_cascade,
        })
        .collect();

    proto::MoteSnapshot {
        mote_id: id.as_bytes().to_vec(),
        state: map_state(state) as i32,
        nd_class: projection
            .nondeterminism_of(&id)
            .map_or(proto::NdClass::Unspecified as i32, |c| {
                proto::NdClass::from(c) as i32
            }),
        promotion: map_promotion(projection.promotion_state(&id)) as i32,
        result_ref: projection.result_ref_of(&id).map(|r| r.0.to_vec()),
        warrant_ref: projection.warrant_ref_of(&id).map(|r| r.0.to_vec()),
        mote_def_hash: def_hashes.get(&id).map_or_else(Vec::new, |h| h.to_vec()),
        committed_seq: projection.committed_seq_of(&id),
        parents,
        // Opaque committed CriticVerdict bytes — never deserialized server-side.
        // Frozen in the wire shape; populated when the promotion path is wired.
        verdict: None,
        anomaly: anomalies.get(&id).map(|k| map_anomaly(*k) as i32),
    }
}

const fn map_state(state: MoteState) -> proto::MoteSnapshotState {
    match state {
        MoteState::Pending => proto::MoteSnapshotState::Pending,
        MoteState::Scheduled => proto::MoteSnapshotState::Scheduled,
        MoteState::Committed => proto::MoteSnapshotState::Committed,
        MoteState::Failed => proto::MoteSnapshotState::Failed,
        MoteState::Repudiated => proto::MoteSnapshotState::Repudiated,
        MoteState::Inconsistent => proto::MoteSnapshotState::Inconsistent,
    }
}

const fn map_promotion(state: PromotionState) -> proto::PromotionState {
    match state {
        PromotionState::NotApplicable => proto::PromotionState::NotApplicable,
        PromotionState::Unpromoted => proto::PromotionState::Unpromoted,
        PromotionState::Promoted => proto::PromotionState::Promoted,
    }
}

const fn map_anomaly(kind: AnomalyKind) -> proto::MoteAnomaly {
    match kind {
        AnomalyKind::EffectStagedThenRepudiatedNoCommitted => {
            proto::MoteAnomaly::EffectStagedThenRepudiatedNoCommitted
        }
        AnomalyKind::QuarantinedAtLeastOnceEffect => {
            proto::MoteAnomaly::QuarantinedAtLeastOnceEffect
        }
    }
}

#[cfg(test)]
mod tests {
    use kx_content::ContentRef;
    use kx_journal::{InMemoryJournal, Journal, JournalEntry};
    use kx_mote::{MoteDefHash, MoteId, NdClass};

    use super::{build_view, map_anomaly, map_promotion, map_state};
    use crate::reader::ReadOnly;
    use kx_proto::proto;

    const INSTANCE: [u8; 16] = [0x11; 16];

    fn journal_with(n: u32) -> InMemoryJournal {
        let journal = InMemoryJournal::new();
        journal
            .append(JournalEntry::RunRegistered {
                instance_id: INSTANCE,
                recipe_fingerprint: [0x22; 32],
                ts: 0,
                seq: 0,
            })
            .unwrap();
        for i in 0..n {
            let mut b = [0u8; 32];
            b[..4].copy_from_slice(&i.to_le_bytes());
            journal
                .append(JournalEntry::Committed {
                    mote_id: MoteId::from_bytes(b),
                    idempotency_key: b,
                    seq: 0,
                    nondeterminism: NdClass::Pure,
                    result_ref: ContentRef::from_bytes(b),
                    parents: smallvec::SmallVec::new(),
                    warrant_ref: ContentRef::from_bytes([0xaa; 32]),
                    mote_def_hash: MoteDefHash::from_bytes(b),
                })
                .unwrap();
        }
        journal
    }

    #[test]
    fn view_is_monotonic_in_at_seq() {
        let reader = ReadOnly::new(journal_with(5));
        // Before the RunRegistered entry (seq 1) the run is not yet established,
        // so the ownership check uniformly denies (no oracle). From seq 1 on, the
        // mote count is monotonic non-decreasing as more entries are folded.
        assert!(matches!(
            build_view(&reader, INSTANCE, Some(0)),
            Err(crate::GatewayError::NotAuthorized)
        ));
        let mut prev = 0usize;
        for at in 1..=6 {
            let view = build_view(&reader, INSTANCE, Some(at)).unwrap();
            assert!(
                view.motes.len() >= prev,
                "folding more entries must never drop motes"
            );
            prev = view.motes.len();
        }
        // At head, every committed mote is rendered.
        let full = build_view(&reader, INSTANCE, None).unwrap();
        assert_eq!(full.motes.len(), 5);
        assert_eq!(full.current_seq, 6);
    }

    #[test]
    fn wrong_instance_is_not_authorized() {
        let reader = ReadOnly::new(journal_with(3));
        assert!(matches!(
            build_view(&reader, [0x99; 16], None),
            Err(crate::GatewayError::NotAuthorized)
        ));
    }

    #[test]
    fn enum_maps_never_emit_unspecified() {
        use kx_projection::{AnomalyKind, MoteState, PromotionState};
        for st in [
            MoteState::Pending,
            MoteState::Scheduled,
            MoteState::Committed,
            MoteState::Failed,
            MoteState::Repudiated,
            MoteState::Inconsistent,
        ] {
            assert_ne!(map_state(st), proto::MoteSnapshotState::Unspecified);
        }
        for pr in [
            PromotionState::NotApplicable,
            PromotionState::Unpromoted,
            PromotionState::Promoted,
        ] {
            assert_ne!(map_promotion(pr), proto::PromotionState::Unspecified);
        }
        for an in [
            AnomalyKind::EffectStagedThenRepudiatedNoCommitted,
            AnomalyKind::QuarantinedAtLeastOnceEffect,
        ] {
            assert_ne!(map_anomaly(an), proto::MoteAnomaly::Unspecified);
        }
    }
}
