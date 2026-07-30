//! The Delta-style sharing manifest: a recipe-as-product whose identity is
//! reproducible by reference — share the recipe + seed, regenerate byte-identical
//! data, verify by `ManifestId`.
#![allow(clippy::unwrap_used)]

use kx_content::ContentRef;
use kx_dataset::{ContentSchema, Dataset, TypedRef};
use kx_mote::{LogicRef, ModelId, ToolName};
use kx_workflow::{compile, permissive_warrant, synthesis_pipeline, transform, Manifest};

fn model() -> ModelId {
    ModelId("local".into())
}

fn pipeline() -> kx_workflow::WorkflowDef {
    synthesis_pipeline(
        7,
        model(),
        ToolName("demo".into()),
        LogicRef::from_bytes([1; 32]),
        LogicRef::from_bytes([2; 32]),
        LogicRef::from_bytes([3; 32]),
    )
    .unwrap()
}

#[test]
fn recipe_manifest_is_reproducible_by_reference() {
    // Compile the same recipe twice (as a recipient on another machine would) →
    // identical compiled DAG → identical ManifestId. This IS the recipe-as-product
    // guarantee: share the recipe + seed, regenerate byte-identically.
    let a = Manifest::recipe(&compile(&pipeline()).unwrap(), 7);
    let b = Manifest::recipe(&compile(&pipeline()).unwrap(), 7);
    assert_eq!(a, b);
    assert_eq!(a.id(), b.id());
    assert_eq!(a.mote_ids.len(), 3);
    assert!(a.dataset_id.is_none());
}

#[test]
fn manifest_id_is_sensitive_to_seed_recipe_and_corpus() {
    let base = Manifest::recipe(&compile(&pipeline()).unwrap(), 7);

    // Different seed → different manifest (seed folds into entrypoint identity).
    let other_seed = Manifest {
        workflow_seed: 8,
        ..base.clone()
    };
    assert_ne!(base.id(), other_seed.id());

    // Pinning a produced corpus changes identity.
    let dataset = Dataset::new(
        vec![TypedRef {
            content_ref: ContentRef::of(b"row"),
            schema: ContentSchema::Blob,
        }],
        vec![],
    );
    let with_corpus = base.clone().with_dataset(dataset.id());
    assert_ne!(base.id(), with_corpus.id());
    assert_eq!(with_corpus.dataset_id, Some(dataset.id()));

    // The pinned-corpus manifest is itself reproducible.
    let with_corpus2 =
        Manifest::recipe(&compile(&pipeline()).unwrap(), 7).with_dataset(dataset.id());
    assert_eq!(with_corpus.id(), with_corpus2.id());
}

/// The AUTHORITY axis of the recipe identity.
///
/// A step warrant is not part of `MoteDef`, so a warrant-only change leaves every
/// `MoteId` untouched — this test asserts that the *recipe* identity nonetheless moves.
/// Without it, a changed warrant produced different body bytes under an unchanged
/// `ManifestId`, which the body ledger refuses as an immutability violation on the
/// serve's startup path (see `kx_gateway::provision`'s
/// `a_react_warrant_change_survives_on_an_already_seeded_state_dir`).
#[test]
fn manifest_id_is_sensitive_to_the_step_warrant() {
    // Two one-step workflows identical in every respect except one warrant field —
    // the smallest possible authority change.
    let step_with = |max_output_tokens: u32| {
        let mut warrant = permissive_warrant(model());
        warrant.model_route.max_output_tokens = max_output_tokens;
        let mut wf = kx_workflow::WorkflowDef::new(7);
        wf.add_step(transform(
            LogicRef::from_bytes([1; 32]),
            model(),
            warrant,
            ToolName("demo".into()),
        ));
        wf
    };
    let base_def = step_with(512);
    let widened_def = step_with(4_096);

    let base = Manifest::recipe(&compile(&base_def).unwrap(), 7);
    let widened = Manifest::recipe(&compile(&widened_def).unwrap(), 7);

    // The COMPUTATION is identical — this is what makes the case subtle, and it is
    // why the warrants had to be folded in explicitly rather than riding MoteId.
    assert_eq!(
        base.mote_ids, widened.mote_ids,
        "a warrant-only change must NOT move any MoteId — two runs differing only in \
         authority are the same computation"
    );
    assert_ne!(
        base.step_warrant_refs, widened.step_warrant_refs,
        "...but the warrant refs must differ"
    );
    assert_ne!(
        base.id(),
        widened.id(),
        "a warrant-only change MUST move the recipe identity, or the body ledger will \
         refuse the new bytes under the old id and fail the serve boot"
    );

    // Same warrant, same id — the fold is a function of the warrant, not of compiling twice.
    assert_eq!(
        Manifest::recipe(&compile(&widened_def).unwrap(), 7).id(),
        widened.id(),
        "the fold is deterministic"
    );
}
