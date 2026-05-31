//! Private helpers shared by [`crate::Projection`] and [`crate::Snapshot`]:
//! transitive-consumers BFS walk, ready-set computation, 3c promotion-state
//! stub.

use std::collections::{BTreeSet, VecDeque};

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
            // trivially. The check is in the contract so it activates when the
            // executor (P1.9) wires the MoteDef registry.
            if let Some(pinfo) = state.motes.get(&p.parent_id) {
                if let Some(committed) = &pinfo.committed {
                    if committed.nondeterminism == NdClass::WorldMutating {
                        match promotion(state, &p.parent_id) {
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
