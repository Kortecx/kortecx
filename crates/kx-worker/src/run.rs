//! Running a leased PURE Mote through the hosted executor.

use kx_content::{ContentRef, SharedContent};
use kx_executor::{run_pure_mote, MoteExecutor, ResourceManager};
use kx_journal::InMemoryJournal;
use kx_mote::{Mote, NdClass};
use kx_warrant::WarrantSpec;
use kx_work_cache::{work_fingerprint, WorkCache};

use crate::error::WorkerError;

/// Run a PURE Mote via [`run_pure_mote`] (kx-executor, VERBATIM) and return its
/// `result_ref`, consulting an optional cross-run work cache AROUND the unchanged
/// executor kernel.
///
/// The executor's commit protocol appends a `Proposed` + `Committed` pair to the
/// `Journal` it is given; the worker hands it a **throwaway** [`InMemoryJournal`]
/// so it never touches the coordinator's durable journal (D40 sole-writer). The
/// local seq is meaningless and discarded — only the body's `result_ref` is real
/// (and is what the worker PROPOSES via `ReportCommit`).
///
/// **The work cache is a worker-side layer, never a kernel edit** (the execution
/// kernel `kx-executor/src` is frozen — the thesis-test guard). When `work_cache` is
/// `Some`, a PURE **child** Mote (its `input_data_id` is content-derived, hence
/// run-independent) whose `(mote_def_hash, input_data_id)` was already computed in ANY
/// run is served from the cache — the executor is not invoked and the cached
/// `result_ref` is proposed directly. On a miss the unchanged [`run_pure_mote`]
/// computes it and the result is populated. `content_store` is the serve's shared store,
/// used only to confirm a cached ref's bytes still exist (GC guard) before serving.
/// `None` for both ⇒ byte-identical to the pre-cache worker. WorldMutating work never
/// reaches here (it dispatches through `run_wm`), so a real effect can never be cached.
pub(crate) fn run_pure<E, R>(
    mote: &Mote,
    warrant: &WarrantSpec,
    executor: &E,
    resource_manager: &R,
    work_cache: Option<&dyn WorkCache>,
    content_store: Option<&dyn SharedContent>,
) -> Result<ContentRef, WorkerError>
where
    E: MoteExecutor + ?Sized,
    R: ResourceManager + ?Sized,
{
    // Cache-eligible iff PURE + a CHILD (content-derived input identity). `graph_position`
    // is deliberately excluded from the key, which is what makes it shared across runs.
    let cache_fp =
        (work_cache.is_some() && mote.nd_class() == NdClass::Pure && !mote.parents.is_empty())
            .then(|| work_fingerprint(NdClass::Pure, &mote.def.hash(), &mote.input_data_id));

    // Read hook: serve a prior run's result iff present AND its bytes still exist.
    if let (Some(cache), Some(fp)) = (work_cache, cache_fp.as_ref()) {
        if let Some(cached_ref) = cache.lookup(fp) {
            if content_store.is_none_or(|s| s.contains(&cached_ref)) {
                return Ok(cached_ref);
            }
        }
    }

    // Miss (or no cache): compute through the UNCHANGED executor kernel.
    let scratch = InMemoryJournal::new();
    let commit = run_pure_mote(mote, warrant, &scratch, resource_manager, executor)?;

    // Populate (first-writer-wins; infallible from our view).
    if let (Some(cache), Some(fp)) = (work_cache, cache_fp) {
        cache.insert(fp, commit.result_ref, NdClass::Pure, mote.id);
    }
    Ok(commit.result_ref)
}

/// Run a coordinator-materialized ReAct TURN (PR-2d-2) — a ROND,
/// `IdempotentByConstruction`, prompt-carrying model Mote (the identity-bearing
/// `REACT_TURN_KEY` marker, empty `tool_contract`) — DIRECTLY through the
/// hosted executor and return its `result_ref` to PROPOSE via `ReportCommit`.
///
/// A turn fits NEITHER existing worker arm: it is not PURE (the frozen
/// `run_pure_mote` enforces the class), and the broker arm (`run_wm`) resolves a
/// capability from `tool_contract` — a turn deliberately declares none (it
/// PROPOSES; the separate observation Mote fires). In the HARNESS the model
/// lives behind the broker (`ModelBroker` runs prompt-carrying ROND Motes); in
/// serve it lives behind the EXECUTOR (`ModelRouterExecutor`, whose react arm
/// decodes + fences the output pre-commit), so the distributed mirror is a
/// direct `executor.run`. Dispatch semantics match `run_wm`'s
/// `IdempotentByConstruction` arm: fire directly (no `EffectStaged` — a greedy
/// decode is serve-once via the coordinator's first-wins commit dedup, R49) and
/// propose the staged `result_ref`. Warrant ceilings are enforced INSIDE the
/// model dispatch (`inference_params_from_mote` refuses a widening, D35).
///
/// **Also serves the T-AGENT2 LLM-JUDGE critic** (any ReadOnlyNondet model Mote
/// dispatched directly through the executor, never the broker): the executor's
/// `run_judge` arm grades the producer + commits a `CriticVerdict`. The function
/// is intentionally generic — it is `executor.run` plus the result-ref extraction.
pub(crate) fn run_react_turn<E>(
    mote: &Mote,
    warrant: &WarrantSpec,
    executor: &E,
) -> Result<ContentRef, WorkerError>
where
    E: MoteExecutor + ?Sized,
{
    // A react turn never carries an environment_ref (minimal-base sandbox).
    let result = executor
        .run(mote, warrant, None)
        .map_err(kx_executor::LifecycleError::from)?;
    Ok(result.result_ref)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use kx_content::InMemoryContentStore;
    use kx_executor::{LocalResourceManager, TestMoteExecutor};
    use kx_mote::{
        EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, MoteDef, MoteId,
        ParentRef, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use kx_warrant::WarrantSpec;
    use kx_work_cache::InMemoryWorkCache;
    use smallvec::SmallVec;

    fn pure_def() -> MoteDef {
        MoteDef {
            critic_check: None,
            logic_ref: LogicRef::from_bytes([1; 32]),
            model_id: ModelId("local".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: kx_mote::InferenceParams::default(),
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        }
    }

    /// A PURE **child** mote (non-empty parents ⇒ cache-eligible). `graph_seed` varies
    /// `graph_position` (⇒ different `MoteId`) while `input_data_id` is held fixed — the
    /// shape of identical work across two different runs.
    fn child_mote(graph_seed: u8) -> Mote {
        let mut parents: SmallVec<[ParentRef; 4]> = SmallVec::new();
        parents.push(ParentRef {
            parent_id: MoteId::from_bytes([9; 32]),
            edge: EdgeMeta::data(),
        });
        Mote::new(
            pure_def(),
            InputDataId::from_bytes([5; 32]),
            GraphPosition(vec![graph_seed]),
            parents,
        )
    }

    /// A PURE **entrypoint** mote (empty parents ⇒ per-run-seed input identity, NOT
    /// cache-eligible in v1).
    fn entrypoint_mote(graph_seed: u8) -> Mote {
        Mote::new(
            pure_def(),
            InputDataId::from_bytes([5; 32]),
            GraphPosition(vec![graph_seed]),
            SmallVec::new(),
        )
    }

    fn counting_executor() -> (TestMoteExecutor, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let ex = TestMoteExecutor::new(move |_m, _w| {
            c.fetch_add(1, Ordering::SeqCst);
            ContentRef::of(b"worker-pure-result")
        });
        (ex, calls)
    }

    #[test]
    fn run_pure_serves_across_runs_from_a_shared_cache() {
        let cache = InMemoryWorkCache::new();
        let (executor, calls) = counting_executor();
        let rm = LocalResourceManager::dev_defaults();
        let w = WarrantSpec::default();

        let a = child_mote(1);
        let b = child_mote(2);
        assert_ne!(a.id, b.id);

        // Run A (this "run") computes and populates the shared cache.
        let r1 = run_pure(&a, &w, &executor, &rm, Some(&cache), None).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Run B (a DIFFERENT run) leases byte-identical PURE work → served, not recomputed.
        let r2 = run_pure(&b, &w, &executor, &rm, Some(&cache), None).unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the worker served run B's PURE result from the cross-run cache"
        );
        assert_eq!(r1, r2);
    }

    #[test]
    fn run_pure_without_cache_is_unchanged() {
        let (executor, calls) = counting_executor();
        let rm = LocalResourceManager::dev_defaults();
        let w = WarrantSpec::default();
        run_pure(&child_mote(1), &w, &executor, &rm, None, None).unwrap();
        run_pure(&child_mote(2), &w, &executor, &rm, None, None).unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "no cache ⇒ each run computes (byte-identical to pre-cache worker)"
        );
    }

    #[test]
    fn entrypoint_motes_are_not_cross_run_cached() {
        let cache = InMemoryWorkCache::new();
        let (executor, calls) = counting_executor();
        let rm = LocalResourceManager::dev_defaults();
        let w = WarrantSpec::default();
        run_pure(&entrypoint_mote(1), &w, &executor, &rm, Some(&cache), None).unwrap();
        run_pure(&entrypoint_mote(2), &w, &executor, &rm, Some(&cache), None).unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "entrypoint motes are excluded (each recomputes)"
        );
        assert!(cache.is_empty(), "entrypoint work is never inserted");
    }

    #[test]
    fn gc_evicted_cached_ref_falls_back_to_recompute() {
        let cache = InMemoryWorkCache::new();
        let a = child_mote(1);
        let fp = work_fingerprint(NdClass::Pure, &a.def.hash(), &a.input_data_id);
        // Seed the cache with a ref whose bytes are NOT in the store.
        let bogus = ContentRef::of(b"evicted-bytes");
        cache.insert(fp, bogus, NdClass::Pure, MoteId::from_bytes([0; 32]));

        let store = InMemoryContentStore::new(); // empty ⇒ does not contain `bogus`
        let (executor, calls) = counting_executor();
        let rm = LocalResourceManager::dev_defaults();
        let w = WarrantSpec::default();

        let got = run_pure(
            &a,
            &w,
            &executor,
            &rm,
            Some(&cache),
            Some(&store as &dyn SharedContent),
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "GC'd bytes ⇒ recompute");
        assert_eq!(got, ContentRef::of(b"worker-pure-result"));
        assert_ne!(got, bogus, "the stale ref was NOT served");
    }
}
