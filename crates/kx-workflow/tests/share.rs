//! The Delta-style sharing manifest: a recipe-as-product whose identity is
//! reproducible by reference — share the recipe + seed, regenerate byte-identical
//! data, verify by `ManifestId`.
#![allow(clippy::unwrap_used)]

use kx_content::ContentRef;
use kx_dataset::{ContentSchema, Dataset, TypedRef};
use kx_mote::{LogicRef, ModelId, ToolName};
use kx_workflow::{compile, synthesis_pipeline, Manifest};

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
