//! PR-2c-3 critic-live — `Projection::ready_set_auto` exit-gate tests.
//!
//! Builds a real `producer (WORLD-MUTATING) → critic → consumer` DAG through the
//! public `register_mote` + `fold` surface and asserts the live `kx serve` gate
//! behaviour end-to-end: a WM producer's consumer is withheld until its critic
//! commits a `Valid` verdict, the gate is FAIL-CLOSED with no verdict lookup, and a
//! critic-free run is byte-for-byte the ungated `ready_set` (zero gate cost).

#![cfg(test)]

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_critic_types::{CheckKind, CriticReason, CriticVerdict};
use kx_journal::{JournalEntry, ParentEntry};
use kx_mote::{EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use smallvec::SmallVec;

use crate::promotion::ContentStoreVerdicts;
use crate::{Projection, RegisterMote};

fn mid(b: u8) -> MoteId {
    MoteId::from_bytes([b; 32])
}

fn data_parent(id: MoteId) -> ParentRef {
    ParentRef {
        parent_id: id,
        edge: EdgeMeta::data(),
    }
}

fn register(
    id: MoteId,
    nd: NdClass,
    critic_for: Option<MoteId>,
    parents: &[ParentRef],
) -> RegisterMote {
    RegisterMote {
        mote_id: id,
        nd_class: nd,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for,
        is_topology_shaper: false,
        parents: parents.iter().copied().collect(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
    }
}

fn commit(
    id: MoteId,
    seq: u64,
    nd: NdClass,
    result_ref: ContentRef,
    parents: &[ParentRef],
) -> JournalEntry {
    let pe: SmallVec<[ParentEntry; 4]> = parents.iter().map(ParentEntry::from_parent_ref).collect();
    JournalEntry::Committed {
        mote_id: id,
        idempotency_key: *id.as_bytes(),
        seq,
        nondeterminism: nd,
        result_ref,
        parents: pe,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
    }
}

fn invalid() -> CriticVerdict {
    CriticVerdict::Invalid {
        reason: CriticReason::Unparseable {
            check: CheckKind::Schema,
            at_offset: 0,
        },
    }
}

/// Build the `producer(WM) → {critic, consumer}` DAG. The critic commits `verdict`
/// (its `result_ref` is the encoded verdict bytes in `store`); the consumer is left
/// Pending (a Data edge to the producer). Returns `(projection, consumer_id)`.
fn dag_with_critic(store: &InMemoryContentStore, verdict: &CriticVerdict) -> (Projection, MoteId) {
    let (producer, critic, consumer) = (mid(1), mid(2), mid(3));
    let mut p = Projection::new();
    // Producer: WORLD-MUTATING, committed.
    p.register_mote(register(producer, NdClass::WorldMutating, None, &[]));
    let producer_out = store.put(b"the producer output").unwrap();
    p.fold(&commit(
        producer,
        1,
        NdClass::WorldMutating,
        producer_out,
        &[],
    ))
    .unwrap();
    // Critic: Pure, critic_for=producer, Data edge to producer, committed verdict.
    p.register_mote(register(
        critic,
        NdClass::Pure,
        Some(producer),
        &[data_parent(producer)],
    ));
    let verdict_ref = store.put(&verdict.encode()).unwrap();
    p.fold(&commit(
        critic,
        2,
        NdClass::Pure,
        verdict_ref,
        &[data_parent(producer)],
    ))
    .unwrap();
    // Consumer: Pure, Data edge to producer, NOT committed ⇒ Pending.
    p.register_mote(register(
        consumer,
        NdClass::Pure,
        None,
        &[data_parent(producer)],
    ));
    (p, consumer)
}

#[test]
fn valid_verdict_promotes_consumer() {
    let store = InMemoryContentStore::new();
    let (p, consumer) = dag_with_critic(&store, &CriticVerdict::Valid);
    let verdicts = ContentStoreVerdicts::new(store);
    assert!(
        p.ready_set_auto(Some(&verdicts)).contains(&consumer),
        "a Valid critic verdict must PROMOTE the WM producer's consumer"
    );
}

#[test]
fn invalid_verdict_withholds_consumer() {
    let store = InMemoryContentStore::new();
    let (p, consumer) = dag_with_critic(&store, &invalid());
    let verdicts = ContentStoreVerdicts::new(store);
    assert!(
        !p.ready_set_auto(Some(&verdicts)).contains(&consumer),
        "an Invalid critic verdict must WITHHOLD the consumer (fail-closed exit gate)"
    );
}

#[test]
fn no_verdict_lookup_withholds_when_critic_declared() {
    // B2 fail-closed: a critic is declared, but no content store resolves its
    // verdict ⇒ the gate withholds (never promotes blind). `None` lookup.
    let store = InMemoryContentStore::new();
    let (p, consumer) = dag_with_critic(&store, &CriticVerdict::Valid);
    assert!(p.has_declared_critic(), "the DAG declares a critic");
    assert!(
        !p.ready_set_auto(None).contains(&consumer),
        "a declared critic with no verdict lookup must WITHHOLD (fail-closed)"
    );
}

#[test]
fn a_pending_critic_is_not_gated_by_its_own_producer() {
    // Deadlock-avoidance: a critic IS the gate, not a gated consumer. With its WM
    // producer committed but UNPROMOTED (the verdict is exactly what this critic will
    // produce), the critic must still be ready — else the producer could never be
    // promoted. A NON-critic consumer of the same producer stays withheld.
    let store = InMemoryContentStore::new();
    let (producer, critic, consumer) = (mid(1), mid(2), mid(3));
    let mut p = Projection::new();
    p.register_mote(register(producer, NdClass::WorldMutating, None, &[]));
    let producer_out = store.put(b"out").unwrap();
    p.fold(&commit(
        producer,
        1,
        NdClass::WorldMutating,
        producer_out,
        &[],
    ))
    .unwrap();
    // Critic: PENDING (registered, not committed), critic_for = producer.
    p.register_mote(register(
        critic,
        NdClass::Pure,
        Some(producer),
        &[data_parent(producer)],
    ));
    // Non-critic consumer of the producer: PENDING.
    p.register_mote(register(
        consumer,
        NdClass::Pure,
        None,
        &[data_parent(producer)],
    ));

    let verdicts = ContentStoreVerdicts::new(store);
    let ready = p.ready_set_auto(Some(&verdicts));
    assert!(
        ready.contains(&critic),
        "the producer's own critic must be ready (it is the gate, not a gated consumer)"
    );
    assert!(
        !ready.contains(&consumer),
        "a non-critic consumer stays withheld until the critic commits Valid"
    );
}

#[test]
fn critic_free_run_is_byte_identical_to_ready_set() {
    // A WM producer with NO critic: has_declared_critic is false, so ready_set_auto
    // takes the ungated ready_set path — the consumer is ready, and the result is
    // identical with Some(verdicts) or None (zero gate cost, digest-invariant).
    let store = InMemoryContentStore::new();
    let (producer, consumer) = (mid(1), mid(3));
    let mut p = Projection::new();
    p.register_mote(register(producer, NdClass::WorldMutating, None, &[]));
    let producer_out = store.put(b"output").unwrap();
    p.fold(&commit(
        producer,
        1,
        NdClass::WorldMutating,
        producer_out,
        &[],
    ))
    .unwrap();
    p.register_mote(register(
        consumer,
        NdClass::Pure,
        None,
        &[data_parent(producer)],
    ));

    assert!(!p.has_declared_critic(), "no critic declared");
    let verdicts = ContentStoreVerdicts::new(store);
    let baseline = p.ready_set();
    assert!(
        baseline.contains(&consumer),
        "ungated: a WM parent with no critic does not withhold"
    );
    assert_eq!(
        p.ready_set_auto(Some(&verdicts)),
        baseline,
        "critic-free run: ready_set_auto(Some) must equal ready_set (byte-identical)"
    );
    assert_eq!(
        p.ready_set_auto(None),
        baseline,
        "critic-free run: ready_set_auto(None) must equal ready_set (byte-identical)"
    );
}
