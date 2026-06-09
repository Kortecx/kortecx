//! Private helpers shared by [`crate::Projection`] and [`crate::Snapshot`]:
//! transitive-consumers BFS walk, ready-set computation, 3c promotion-state
//! stub.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kx_mote::{EdgeKind, MoteId, NdClass};

use crate::enums::{MoteState, PromotionState};
use crate::state::State;

pub(crate) fn transitive_consumers_impl(state: &State, root: &MoteId) -> Vec<MoteId> {
    let mut visited: BTreeSet<MoteId> = BTreeSet::new();
    let mut order: Vec<MoteId> = Vec::new();
    let mut queue: VecDeque<MoteId> = VecDeque::new();
    queue.push_back(*root);

    while let Some(current) = queue.pop_front() {
        // children_of for the current node
        let children = state.children.get(&current).cloned().unwrap_or_default();
        for (child, edge) in children {
            // Cascade rule: data edges always cascade; control edges cascade unless
            // explicitly non_cascade.
            let should_walk = match edge.kind {
                EdgeKind::Data => true,
                EdgeKind::Control => !edge.non_cascade,
            };
            if !should_walk {
                continue;
            }
            if visited.insert(child) {
                order.push(child);
                queue.push_back(child);
            }
        }
    }
    order
}

pub(crate) fn ready_set_impl(state: &State) -> Vec<MoteId> {
    // The pure default: promotion always NotApplicable (the P1 contract). The
    // verdict-aware path is `ready_set_impl_with` (P4.2-3).
    ready_set_impl_with(state, &|s, id| promotion_state_impl(s, id))
}

/// `ready_set_impl` parameterized by the WORLD-MUTATING promotion oracle. The
/// pure `ready_set_impl` passes the `NotApplicable` stub; the P4.2-3 exit gate
/// passes a verdict-reading closure
/// (`crate::promotion::promotion_state_with`).
pub(crate) fn ready_set_impl_with(
    state: &State,
    promotion: &dyn Fn(&State, &MoteId) -> PromotionState,
) -> Vec<MoteId> {
    let mut out: Vec<MoteId> = Vec::new();
    // Per-pass promotion memo (PR-2c-3 critic-live, H2/H3). The WORLD-MUTATING
    // promotion gate is keyed by the PRODUCER, and many pending consumers commonly
    // share the same producer, so the unmemoized oracle recomputes the same verdict
    // scan + content-store decode once per consumer (the hot-path waste the red-team
    // flagged). Computing each producer's promotion state once per `ready_set` pass
    // makes the cost O(distinct WM producers · M) instead of O(consumers · M); the
    // result is byte-identical (a committed verdict is an immutable content-addressed
    // fact), so the demo digest and the P1 contract are untouched. The default
    // `NotApplicable` stub never even reaches here (it returns before any store read).
    let mut promo_memo: BTreeMap<MoteId, PromotionState> = BTreeMap::new();
    for (id, info) in &state.motes {
        if state.state_of_id(id) != MoteState::Pending {
            continue;
        }
        // Parents must all be Committed-and-not-Repudiated.
        let Some(d) = info.declared.as_ref() else {
            // Pending implies declared (registered).
            continue;
        };
        let mut all_parents_committed = true;
        let mut all_wm_parents_promoted = true;
        for p in &d.parents {
            let pstate = state.state_of_id(&p.parent_id);
            if pstate != MoteState::Committed {
                all_parents_committed = false;
                break;
            }
            // WORLD-MUTATING promotion gate (per projection.md §7). In P1 default,
            // promotion_state always returns NotApplicable, so the gate passes
            // trivially. The P4.2-3 verdict-aware oracle (live in `kx serve` as of
            // PR-2c-3) withholds a consumer until every critic of a WM producer
            // commits a `Valid` verdict.
            //
            // EXCEPT the producer's OWN critic: a critic (`critic_for == p.parent_id`)
            // IS the gate, not a gated consumer — it must run to PRODUCE the verdict
            // that promotes the producer. Gating it on the producer's (still-pending)
            // promotion would deadlock (the producer can never be promoted because its
            // critic can never run). A critic of a DIFFERENT WM parent stays gated by
            // that parent. (Live-only effect: under the P1 `NotApplicable` default the
            // branch is never taken, so the demo digest is byte-unchanged.)
            if d.critic_for.as_ref() == Some(&p.parent_id) {
                continue;
            }
            if let Some(pinfo) = state.motes.get(&p.parent_id) {
                if let Some(committed) = &pinfo.committed {
                    if committed.nondeterminism == NdClass::WorldMutating {
                        let promo = *promo_memo
                            .entry(p.parent_id)
                            .or_insert_with(|| promotion(state, &p.parent_id));
                        match promo {
                            PromotionState::Promoted | PromotionState::NotApplicable => {}
                            PromotionState::Unpromoted => {
                                all_wm_parents_promoted = false;
                                break;
                            }
                        }
                    }
                }
            }
        }
        if all_parents_committed && all_wm_parents_promoted {
            out.push(*id);
        }
    }
    out
}

pub(crate) fn promotion_state_impl(_state: &State, _mote_id: &MoteId) -> PromotionState {
    // P1 default per D18: no observable critic-of-producer relationships until
    // P1.9's MoteDef registry wires the lookup. All Motes return NotApplicable.
    PromotionState::NotApplicable
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use kx_content::ContentRef;
    use kx_mote::{EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
    use smallvec::SmallVec;

    use super::ready_set_impl_with;
    use crate::enums::PromotionState;
    use crate::state::{CommittedInfo, DeclaredInfo, MoteInfo, State};

    fn id(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }

    /// PR-2c-3 critic-live (H2/H3): the promotion oracle is memoized PER PASS — many
    /// pending consumers of one WORLD-MUTATING producer compute its promotion state
    /// ONCE, not once per consumer. Proves the gate cost is `O(distinct WM producers)`,
    /// not `O(consumers)`, on the lease hot path (the scalability guarantee), and that
    /// the memo is behaviour-preserving (every consumer still becomes ready).
    #[test]
    fn promotion_oracle_is_memoized_per_pass() {
        let producer = id(1);
        let mut state = State::default();
        // A committed WORLD-MUTATING producer.
        state.motes.insert(
            producer,
            MoteInfo {
                declared: Some(DeclaredInfo {
                    nd_class: NdClass::WorldMutating,
                    effect_pattern: EffectPattern::StageThenCommit,
                    critic_for: None,
                    is_topology_shaper: false,
                    parents: SmallVec::new(),
                    warrant_ref: ContentRef::from_bytes([0; 32]),
                }),
                committed: Some(CommittedInfo {
                    seq: 1,
                    result_ref: ContentRef::from_bytes([9; 32]),
                    nondeterminism: NdClass::WorldMutating,
                    parents_in_entry: SmallVec::new(),
                    warrant_ref: ContentRef::from_bytes([0; 32]),
                    mote_def_hash: MoteDefHash::from_bytes([0; 32]),
                    repudiated: false,
                }),
                ..Default::default()
            },
        );
        // 16 pending consumers, each a Data child of the one producer.
        let consumers = 16u8;
        for c in 2..2 + consumers {
            state.motes.insert(
                id(c),
                MoteInfo {
                    declared: Some(DeclaredInfo {
                        nd_class: NdClass::Pure,
                        effect_pattern: EffectPattern::IdempotentByConstruction,
                        critic_for: None,
                        is_topology_shaper: false,
                        parents: SmallVec::from_vec(vec![ParentRef {
                            parent_id: producer,
                            edge: EdgeMeta::data(),
                        }]),
                        warrant_ref: ContentRef::from_bytes([0; 32]),
                    }),
                    committed: None, // Pending
                    ..Default::default()
                },
            );
        }

        let calls = Cell::new(0u32);
        let ready = ready_set_impl_with(&state, &|_s, _id| {
            calls.set(calls.get() + 1);
            PromotionState::Promoted
        });

        assert_eq!(
            calls.get(),
            1,
            "the WM producer's promotion is computed ONCE for all {consumers} consumers (memoized)"
        );
        assert_eq!(
            ready.len(),
            consumers as usize,
            "every consumer of a Promoted producer becomes ready (behaviour-preserving)"
        );
    }
}
