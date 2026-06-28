//! Fail-closed gate tests for the multi-modal IMAGE dispatch path (PR-2).
//!
//! Every case here asserts a gate that fires BEFORE the loaded-model cache is
//! touched — so no GGUF, no FFI, and no owner thread is needed; the tests are
//! fast and deterministic. The real image→text inference (gates pass → decode →
//! generate) is exercised by the `model-smoke-test-multimodal` gate against a
//! real VLM.
//!
//! Gate order under test (`LlamaInferenceBackend::dispatch`): warrant route →
//! scope → resolve descriptor → [image] capability → content store bound → ref
//! resolves → size cap → image sniff → mmproj present → cache. (RC2 removed the
//! former leading grammar-reservation gate — grammar is now honored at
//! sampler-build time, never rejected here.)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_inference::{
    InferenceBackend, InferenceError, InferenceInput, InferenceParams, LlamaInferenceBackend,
    MEDIA_MARKER,
};
use kx_model_store::{ModelDescriptor, ModelRegistry};
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

fn warrant(model_id: ModelId, mem_bytes: u64) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef([0u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id,
            max_input_tokens: 4096,
            max_output_tokens: 512,
            max_calls: 100,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// A small but valid-magic PNG payload (sniff recognizes it; never decoded in
/// these gate tests).
fn fake_png(n: usize) -> Vec<u8> {
    let mut v = PNG_MAGIC.to_vec();
    v.resize(n.max(PNG_MAGIC.len()), 0u8);
    v
}

fn image_backend(
    id: &ModelId,
    store: Option<Arc<InMemoryContentStore>>,
    with_mmproj: bool,
) -> LlamaInferenceBackend {
    let mut registry = ModelRegistry::new();
    let descriptor = if with_mmproj {
        ModelDescriptor::image(id.clone(), "/m/vlm.gguf", "/m/vlm-mmproj.gguf", 4096)
    } else {
        // Image modality but NO projector (a misconfiguration the dispatch
        // must catch): build via `new` with an Image modality and mmproj=None.
        let mut mods = SmallVec::new();
        mods.push(kx_model_store::Modality::Image);
        ModelDescriptor::new(id.clone(), "/m/vlm.gguf", None, mods, 4096)
    };
    registry.register(descriptor).unwrap();
    let backend = LlamaInferenceBackend::with_resolver(Arc::new(registry));
    match store {
        Some(s) => backend.with_content_store(s),
        None => backend,
    }
}

fn multimodal_input(refs: &[ContentRef]) -> InferenceInput {
    InferenceInput::Multimodal {
        text: format!("{MEDIA_MARKER}describe this image"),
        content_refs: SmallVec::from_slice(refs),
    }
}

#[test]
fn text_only_model_rejects_image_request() {
    let id = ModelId("text-only".into());
    let store = Arc::new(InMemoryContentStore::new());
    let r = store.put(&fake_png(64)).unwrap();
    // A plain text model (with_model) does not declare Image.
    let backend = LlamaInferenceBackend::with_model(id.clone(), PathBuf::from("/m/text.gguf"))
        .with_content_store(store);
    let err = backend
        .dispatch(
            &id,
            &multimodal_input(&[r]),
            &InferenceParams::default(),
            &warrant(id.clone(), 1 << 30),
        )
        .expect_err("text-only model must reject an image request");
    assert!(
        matches!(err, InferenceError::Unsupported { .. }),
        "got {err:?}"
    );
}

#[test]
fn image_model_without_content_store_is_unsupported() {
    let id = ModelId("vlm".into());
    let backend = image_backend(&id, None, true);
    let r = ContentRef([7u8; 32]);
    let err = backend
        .dispatch(
            &id,
            &multimodal_input(&[r]),
            &InferenceParams::default(),
            &warrant(id.clone(), 1 << 30),
        )
        .expect_err("no content store bound must be Unsupported");
    assert!(
        matches!(err, InferenceError::Unsupported { .. }),
        "got {err:?}"
    );
}

#[test]
fn unresolvable_ref_is_content_store_miss() {
    let id = ModelId("vlm".into());
    let store = Arc::new(InMemoryContentStore::new());
    let backend = image_backend(&id, Some(store), true);
    let missing = ContentRef([42u8; 32]);
    let err = backend
        .dispatch(
            &id,
            &multimodal_input(&[missing]),
            &InferenceParams::default(),
            &warrant(id.clone(), 1 << 30),
        )
        .expect_err("missing ref must be ContentStoreMiss");
    match err {
        InferenceError::ContentStoreMiss { content_ref } => assert_eq!(content_ref, missing),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn oversized_image_fails_closed_with_scope_violation() {
    let id = ModelId("vlm".into());
    let store = Arc::new(InMemoryContentStore::new());
    let r = store.put(&fake_png(100)).unwrap();
    let backend = image_backend(&id, Some(store), true);
    // mem_bytes = 10 < 100-byte image ⇒ rejected before the decoder ever runs.
    let err = backend
        .dispatch(
            &id,
            &multimodal_input(&[r]),
            &InferenceParams::default(),
            &warrant(id.clone(), 10),
        )
        .expect_err("oversized image must be rejected pre-decode");
    match err {
        InferenceError::ScopeViolation {
            field,
            requested,
            ceiling,
        } => {
            assert_eq!(field, "image_bytes");
            assert_eq!(requested, 100);
            assert_eq!(ceiling, 10);
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn non_image_bytes_are_unsupported_in_pr2() {
    let id = ModelId("vlm".into());
    let store = Arc::new(InMemoryContentStore::new());
    // Plausible model text — must NOT be mistaken for an image; audio/unknown
    // are reserved for later PRs.
    let r = store.put(b"the chart shows a rising trend").unwrap();
    let backend = image_backend(&id, Some(store), true);
    let err = backend
        .dispatch(
            &id,
            &multimodal_input(&[r]),
            &InferenceParams::default(),
            &warrant(id.clone(), 1 << 30),
        )
        .expect_err("non-image content must be Unsupported");
    assert!(
        matches!(err, InferenceError::Unsupported { .. }),
        "got {err:?}"
    );
}

#[test]
fn image_model_without_projector_is_unsupported() {
    let id = ModelId("vlm-no-mmproj".into());
    let store = Arc::new(InMemoryContentStore::new());
    let r = store.put(&fake_png(64)).unwrap();
    // Image modality declared but mmproj absent: refs resolve fine, then the
    // projector gate fails closed.
    let backend = image_backend(&id, Some(store), false);
    let err = backend
        .dispatch(
            &id,
            &multimodal_input(&[r]),
            &InferenceParams::default(),
            &warrant(id.clone(), 1 << 30),
        )
        .expect_err("image model with no projector must be Unsupported");
    assert!(
        matches!(err, InferenceError::Unsupported { .. }),
        "got {err:?}"
    );
}

#[test]
fn warrant_denies_image_request_before_content_is_touched() {
    // A mismatched warrant route must deny BEFORE any content fetch — authz
    // precedes content handling.
    let id = ModelId("vlm".into());
    let store = Arc::new(InMemoryContentStore::new());
    let r = store.put(&fake_png(64)).unwrap();
    let backend = image_backend(&id, Some(store), true);
    let other_route = warrant(ModelId("a-different-model".into()), 1 << 30);
    let err = backend
        .dispatch(
            &id,
            &multimodal_input(&[r]),
            &InferenceParams::default(),
            &other_route,
        )
        .expect_err("mismatched warrant route must deny");
    assert!(
        matches!(err, InferenceError::WarrantDeniesModel { .. }),
        "got {err:?}"
    );
}
