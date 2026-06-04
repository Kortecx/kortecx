//! Property-based tests for the dispatcher / backend seam — SN-4 v2:
//! at least 3 proptest properties × 64 cases each.
//!
//! Properties exercised:
//!   1. `Unsupported`-on-Multimodal is deterministic — for any prompt,
//!      seed, and content-ref set, the backend returns the SAME error
//!      string. (Reservation semantics are invariant across inputs.)
//!   2. `Unsupported`-on-grammar=Some is deterministic — for any
//!      `Grammar` payload, the backend returns the same error.
//!   3. `WarrantDeniesModel` fires whenever `requested_model_id !=
//!      warrant.model_route.model_id`, regardless of any other field.
//!      (Prefix-monotonic on the model-route axis: changing other
//!      fields cannot unset this refusal.)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::FakeBackend;
use kx_content::ContentRef;
use kx_inference::{
    Grammar, InferenceBackend, InferenceError, InferenceInput, InferenceParams,
    LlamaInferenceBackend,
};
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use proptest::collection::vec as prop_vec;
use proptest::prelude::*;
use smallvec::SmallVec;

fn warrant_with_route(model_id: ModelId, max_output_tokens: u32) -> WarrantSpec {
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
            max_input_tokens: 2048,
            max_output_tokens,
            max_calls: 100,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

fn arbitrary_content_refs() -> impl Strategy<Value = SmallVec<[ContentRef; 4]>> {
    prop_vec(any::<[u8; 32]>().prop_map(ContentRef), 0..=4).prop_map(SmallVec::from_vec)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Property 1 (PR-2, narrowed contract): a Multimodal request against a
    /// **text-only** model is deterministically `Unsupported` — for any text +
    /// content-ref input — because the capability gate (`descriptor.supports
    /// (Image)`) fails closed BEFORE any content fetch or FFI. `with_model`
    /// registers a text-only descriptor, so the model never declares Image.
    /// (Multimodal is no longer *blanket*-reserved — an image-capable model
    /// with a projector + bound store serves it; that path is covered in
    /// `tests/multimodal_dispatch.rs`.)
    #[test]
    fn prop_multimodal_text_only_model_unsupported(
        text in ".{0,256}",
        refs in arbitrary_content_refs(),
    ) {
        let id = ModelId("any-model".into());
        // Register a text-only model so `ModelNotFound` cannot mask the
        // Unsupported error; the capability gate fires once resolved.
        let backend = LlamaInferenceBackend::with_model(
            id.clone(),
            std::path::PathBuf::from("/dev/null"),
        );
        let warrant = warrant_with_route(id.clone(), 512);
        let input = InferenceInput::Multimodal { text, content_refs: refs };
        let params = InferenceParams::default();
        let err = backend.dispatch(&id, &input, &params, &warrant)
            .expect_err("multimodal against a text-only model must be Unsupported");
        let is_unsupported = matches!(err, InferenceError::Unsupported { .. });
        prop_assert!(is_unsupported);
    }

    /// Property 2: `Unsupported` on grammar=Some is deterministic
    /// across any payload string. Same invariance argument as
    /// Property 1 but on the grammar field.
    #[test]
    fn prop_grammar_some_always_unsupported(
        raw in ".{0,256}",
    ) {
        let id = ModelId("any-model".into());
        let backend = LlamaInferenceBackend::with_model(
            id.clone(),
            std::path::PathBuf::from("/dev/null"),
        );
        let warrant = warrant_with_route(id.clone(), 512);
        let input = InferenceInput::Text("hi".into());
        let params = InferenceParams {
            grammar: Some(Grammar::new(raw)),
            ..InferenceParams::default()
        };
        let err = backend.dispatch(&id, &input, &params, &warrant)
            .expect_err("grammar=Some must be Unsupported");
        let is_unsupported_grammar = matches!(err, InferenceError::Unsupported { .. });
        prop_assert!(is_unsupported_grammar);
    }

    /// Property 3: `WarrantDeniesModel` is invariant on every other
    /// dimension. Whenever the requested id and the warrant's route
    /// disagree, the dispatcher refuses — regardless of params,
    /// input, or registry membership. (Mirror of the prefix-
    /// monotonicity-of-refusal property kx-projection already proves
    /// for journal refusal.)
    #[test]
    fn prop_warrant_denies_other_models(
        requested_id in "[a-z]{1,16}",
        warrant_id in "[a-z]{1,16}",
        max_tokens in 1u32..=1024,
    ) {
        prop_assume!(requested_id != warrant_id);
        let req = ModelId(requested_id.clone());
        // Register the requested model on the backend so model-not-
        // found cannot mask the deny.
        let backend = LlamaInferenceBackend::with_model(
            req.clone(),
            std::path::PathBuf::from("/dev/null"),
        );
        let warrant = warrant_with_route(ModelId(warrant_id.clone()), max_tokens);
        let input = InferenceInput::Text("x".into());
        let params = InferenceParams::default();
        let err = backend.dispatch(&req, &input, &params, &warrant)
            .expect_err("mismatched ids must deny");
        let is_warrant_denies = matches!(err, InferenceError::WarrantDeniesModel { .. });
        prop_assert!(is_warrant_denies);
    }

    /// Property 4 (bonus): the FakeBackend, registered for an exact
    /// `ModelId`, always returns Ok for that id and `ModelNotFound`
    /// for any other. Reinforces SN-8 "exact cryptographic equality"
    /// — substring matches do not count.
    #[test]
    fn prop_fake_supports_is_exact(
        registered in "[a-z]{1,8}",
        queried in "[a-z]{1,8}",
    ) {
        let reg_id = ModelId(registered.clone());
        let queried_id = ModelId(queried.clone());
        let backend = FakeBackend::new("f").with_model(reg_id.clone());
        let warrant = warrant_with_route(queried_id.clone(), 512);
        let input = InferenceInput::Text("x".into());
        let params = InferenceParams::default();
        let result = backend.dispatch(&queried_id, &input, &params, &warrant);
        if registered == queried {
            prop_assert!(result.is_ok());
        } else {
            let err = result.expect_err("unrelated id must fail");
            let is_not_found = matches!(err, InferenceError::ModelNotFound { .. });
            prop_assert!(is_not_found);
        }
    }
}
