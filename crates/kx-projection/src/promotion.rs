//! P4.2-3 — the deterministic-critic promotion gate (the **P4 EXIT GATE**).
//!
//! A WORLD-MUTATING producer's downstream consumers are withheld from the ready
//! set until a deterministic critic Mote (declared `critic_for = producer`) has
//! committed a **`Valid`** [`CriticVerdict`]. The verdict is read by
//! content-address from the committed critic's `result_ref` — a pure fold of the
//! journal (the critic-of-producer relationship + the committed ref are folded
//! State) plus a content-addressed lookup (the same shape as the topology
//! materializer, which already composes a [`ContentStore`] beside the fold).
//!
//! **SN-8.** The gate decides on **exact crypto-equality** of the committed
//! verdict (`CriticVerdict::is_valid` over the decoded fact), never a score or
//! similarity. The verdict's evidence is integer-only; no float reaches this
//! decision. The verdict bytes are themselves content-addressed, so a tampered
//! verdict does not decode to a different decision — it fails to decode (treated
//! as "not Valid" → withhold), fail-closed.

use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, SharedContent};
use kx_critic_types::CriticVerdict;
use kx_mote::MoteId;

use crate::enums::PromotionState;
use crate::state::State;

/// Resolves a committed critic's `result_ref` to its [`CriticVerdict`].
///
/// Abstracts the content-store read so `promotion_state_with` stays a pure
/// decision over `State` + verdict lookups (testable without a real store).
pub trait VerdictLookup {
    /// The committed [`CriticVerdict`] at `critic_result_ref`, or `None` if the
    /// bytes are missing or fail to decode (both treated, fail-closed, as "not a
    /// Valid verdict").
    fn verdict(&self, critic_result_ref: &ContentRef) -> Option<CriticVerdict>;
}

/// Production resolver: reads the verdict bytes from a [`ContentStore`] and
/// decodes them with [`CriticVerdict::decode`].
pub struct ContentStoreVerdicts<S> {
    store: S,
}

impl<S: ContentStore> ContentStoreVerdicts<S> {
    /// Wrap a content store as a verdict source.
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S: ContentStore> VerdictLookup for ContentStoreVerdicts<S> {
    fn verdict(&self, r: &ContentRef) -> Option<CriticVerdict> {
        let bytes = self.store.get(r).ok()?;
        CriticVerdict::decode(&bytes).ok()
    }
}

/// Production resolver backed by the type-erased [`SharedContent`] seam
/// (D181.4). Identical read to [`ContentStoreVerdicts`], but over the
/// runtime's shared `Arc<dyn SharedContent>` handle so the commit-time critic
/// gate resolves verdicts through whichever backend (local FS or S3) is wired,
/// without the coordinator naming the concrete store type.
pub struct SharedContentVerdicts {
    store: Arc<dyn SharedContent>,
}

impl SharedContentVerdicts {
    /// Wrap a shared content handle as a verdict source.
    pub fn new(store: Arc<dyn SharedContent>) -> Self {
        Self { store }
    }
}

impl VerdictLookup for SharedContentVerdicts {
    fn verdict(&self, r: &ContentRef) -> Option<CriticVerdict> {
        let bytes = self.store.get(r).ok()?;
        CriticVerdict::decode(&bytes).ok()
    }
}

/// A fail-closed [`VerdictLookup`] that resolves NOTHING — every verdict is
/// treated as missing (⇒ "not Valid" ⇒ withhold). Used by
/// [`crate::Projection::ready_set_auto`] when a critic IS declared in the run but
/// no content store is available to resolve its verdict: the gate withholds the
/// producer's consumers rather than promoting blind. This keeps the exit gate a
/// deterministic pure fold of the journal — it can never fail OPEN because a store
/// handle happened to be absent (PR-2c-3 critic-live, B2).
pub struct NoVerdicts;

impl VerdictLookup for NoVerdicts {
    fn verdict(&self, _critic_result_ref: &ContentRef) -> Option<CriticVerdict> {
        None
    }
}

/// Compute the [`PromotionState`] of `producer_id` against committed critic
/// verdicts. Pure over `state` + the verdict lookups.
///
/// - No critic declared for the producer → [`PromotionState::NotApplicable`]
///   (the producer is ungated; the WM-promotion filter passes it through).
/// - A critic declared but not yet committed (or repudiated) →
///   [`PromotionState::Unpromoted`] (withhold consumers until the gate decides).
/// - **Every** committed critic returns a `Valid` verdict →
///   [`PromotionState::Promoted`].
/// - Any committed critic returns `Invalid`, or its verdict bytes are
///   missing/undecodable → [`PromotionState::Unpromoted`] (fail-closed).
pub(crate) fn promotion_state_with(
    state: &State,
    producer_id: &MoteId,
    lookup: &dyn VerdictLookup,
) -> PromotionState {
    let mut saw_critic = false;
    let mut all_valid = true;
    for info in state.motes.values() {
        let Some(declared) = &info.declared else {
            continue;
        };
        if declared.critic_for.as_ref() != Some(producer_id) {
            continue;
        }
        saw_critic = true;
        match &info.committed {
            // Declared-but-uncommitted, or repudiated: cannot promote yet.
            None => all_valid = false,
            Some(c) if c.repudiated => all_valid = false,
            Some(c) => {
                if !matches!(lookup.verdict(&c.result_ref), Some(v) if v.is_valid()) {
                    all_valid = false;
                }
            }
        }
    }
    if !saw_critic {
        PromotionState::NotApplicable
    } else if all_valid {
        PromotionState::Promoted
    } else {
        PromotionState::Unpromoted
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use kx_critic_types::{CheckKind, CriticReason, CriticVerdict};
    use kx_mote::{EffectPattern, MoteDefHash, MoteId, NdClass};
    use smallvec::SmallVec;

    use super::{promotion_state_with, ContentStoreVerdicts, VerdictLookup};
    use crate::enums::PromotionState;
    use crate::state::{CommittedInfo, DeclaredInfo, MoteInfo, State};

    fn id(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }
    fn cref(b: u8) -> kx_content::ContentRef {
        kx_content::ContentRef::from_bytes([b; 32])
    }
    fn invalid() -> CriticVerdict {
        CriticVerdict::Invalid {
            reason: CriticReason::Unparseable {
                check: CheckKind::Schema,
                at_offset: 0,
            },
        }
    }

    /// A critic Mote declared for `producer`, optionally committed at `ref`.
    fn critic_info(producer: MoteId, committed_ref: Option<kx_content::ContentRef>) -> MoteInfo {
        MoteInfo {
            declared: Some(DeclaredInfo {
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
                critic_for: Some(producer),
                is_topology_shaper: false,
                parents: SmallVec::new(),
                warrant_ref: cref(0),
            }),
            committed: committed_ref.map(|r| CommittedInfo {
                seq: 1,
                result_ref: r,
                nondeterminism: NdClass::Pure,
                parents_in_entry: SmallVec::new(),
                warrant_ref: cref(0),
                mote_def_hash: MoteDefHash::from_bytes([0; 32]),
                repudiated: false,
            }),
            ..Default::default()
        }
    }

    fn state_with(critics: Vec<(u8, MoteInfo)>) -> State {
        let mut s = State::default();
        for (b, info) in critics {
            s.motes.insert(id(b), info);
        }
        s
    }

    struct MockVerdicts(BTreeMap<kx_content::ContentRef, CriticVerdict>);
    impl VerdictLookup for MockVerdicts {
        fn verdict(&self, r: &kx_content::ContentRef) -> Option<CriticVerdict> {
            self.0.get(r).cloned()
        }
    }

    #[test]
    fn no_critic_declared_is_not_applicable() {
        let producer = id(1);
        let state = state_with(vec![]);
        let lookup = MockVerdicts(BTreeMap::new());
        assert_eq!(
            promotion_state_with(&state, &producer, &lookup),
            PromotionState::NotApplicable
        );
    }

    #[test]
    fn committed_valid_verdict_promotes() {
        let producer = id(1);
        let state = state_with(vec![(2, critic_info(producer, Some(cref(9))))]);
        let mut m = BTreeMap::new();
        m.insert(cref(9), CriticVerdict::Valid);
        let lookup = MockVerdicts(m);
        assert_eq!(
            promotion_state_with(&state, &producer, &lookup),
            PromotionState::Promoted
        );
    }

    #[test]
    fn committed_invalid_verdict_withholds() {
        let producer = id(1);
        let state = state_with(vec![(2, critic_info(producer, Some(cref(9))))]);
        let mut m = BTreeMap::new();
        m.insert(cref(9), invalid());
        let lookup = MockVerdicts(m);
        assert_eq!(
            promotion_state_with(&state, &producer, &lookup),
            PromotionState::Unpromoted
        );
    }

    #[test]
    fn declared_but_uncommitted_critic_withholds() {
        let producer = id(1);
        let state = state_with(vec![(2, critic_info(producer, None))]);
        let lookup = MockVerdicts(BTreeMap::new());
        assert_eq!(
            promotion_state_with(&state, &producer, &lookup),
            PromotionState::Unpromoted
        );
    }

    #[test]
    fn missing_or_undecodable_verdict_bytes_fail_closed() {
        // Committed critic but the verdict bytes are absent from the store.
        let producer = id(1);
        let state = state_with(vec![(2, critic_info(producer, Some(cref(9))))]);
        let lookup = MockVerdicts(BTreeMap::new()); // ref(9) absent
        assert_eq!(
            promotion_state_with(&state, &producer, &lookup),
            PromotionState::Unpromoted
        );
    }

    #[test]
    fn all_critics_must_be_valid_to_promote() {
        // Two critics for the same producer; one Invalid → Unpromoted.
        let producer = id(1);
        let state = state_with(vec![
            (2, critic_info(producer, Some(cref(8)))),
            (3, critic_info(producer, Some(cref(9)))),
        ]);
        let mut m = BTreeMap::new();
        m.insert(cref(8), CriticVerdict::Valid);
        m.insert(cref(9), invalid());
        let lookup = MockVerdicts(m);
        assert_eq!(
            promotion_state_with(&state, &producer, &lookup),
            PromotionState::Unpromoted
        );
    }

    #[test]
    fn content_store_verdicts_decodes_real_bytes() {
        // The production resolver round-trips a real encoded verdict.
        use kx_content::{ContentStore, InMemoryContentStore};
        let store = InMemoryContentStore::new();
        let r = store.put(&CriticVerdict::Valid.encode()).unwrap();
        let resolver = ContentStoreVerdicts::new(store);
        assert_eq!(resolver.verdict(&r), Some(CriticVerdict::Valid));
    }
}
