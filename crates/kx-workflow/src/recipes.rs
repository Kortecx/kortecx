//! The **Morphic recipe library**: ready-to-run multi-agent patterns authored as
//! pure data, composed entirely from the existing step builders in
//! [`crate::synthesis`] (`generator` / `transform` / `deterministic_critic`).
//!
//! Each recipe returns a [`WorkflowDef`] that [`compile`](crate::compile)s to a
//! deterministic Mote DAG: pin the `seed` + model + inference params and the
//! recipe re-derives byte-identical `MoteId`s (D50). Recipes are **additive** —
//! they wire existing builders + edges and change no core topology, no
//! `compile` lowering, and no runtime materializer.
//!
//! # Width is bounded (single-level) by construction
//!
//! Every recipe here has a **bounded, authoring-time width** (a fixed number of
//! mappers / workers / attempts / images / turns), so the whole graph is a
//! static, fully-wired DAG. That is the correct single-level form: a
//! runtime-decided, *dynamic* fan-out width is the job of a `topology_shaper`
//! ([`crate::topology_shaper`], exercised by the runtime materializer) — and a
//! true *in-workflow iterative re-shaping* loop (a shaper re-deciding from its
//! own children's verdicts within one run) is the advanced topology kept for the
//! cloud tier. Multi-round iteration on the single-node runtime is expressed by
//! **appending a fresh registered round** (the planner's D76/D77 replanning),
//! never by mutating a committed Mote.
//!
//! # Prompts
//!
//! A recipe wires step *structure*; bind each step's prompt with the
//! [`crate::prompt`] engine (`config_subset[`[`TEMPLATE_KEY`](crate::TEMPLATE_KEY)`]`
//! → [`render_prompts`](crate::render_prompts)) before [`compile`](crate::compile).

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_critic_types::CheckSpec;
use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, LogicRef, ModelId, ToolName, ToolVersion, RETRIEVAL_MODE_KEY,
};

use crate::def::WorkflowDef;
use crate::error::CompileError;
use crate::retrieval::retrieval;
use crate::synthesis::{
    deterministic_critic, generator, permissive_warrant, rewrite_query, transform,
};

/// Which builder a fan-out leaf step uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerKind {
    /// A PURE deterministic mapper ([`crate::transform`]) — `IdempotentByConstruction`.
    Transform,
    /// A READ-ONLY-NONDET sampler ([`crate::generator`]) — `StageThenCommit`.
    Generator,
}

/// The `config_subset` key under which an image-describe step records *which*
/// image it describes — its content ref, baked at authoring time by
/// [`image_batch_describe_reduce`] (one distinct image per describe step). This
/// is the per-step image *association* (identity-bearing → distinct describe
/// identities), **not** the dispatch-time image input: the multi-modal path
/// (PR-2) feeds the model from a describe step's image-sniffed Data-edge *parent*
/// content, so a runnable multi-modal describe supplies the image as a committed
/// parent (a dispatch extension that fetches this ref directly is a future
/// model-harness follow-up).
pub const IMAGE_REF_KEY: &str = "image_ref";

/// Shared fan-in builder: `N` leaf steps of `kind` → one PURE `combine` step,
/// with a Data edge from each leaf to `combine`.
fn fan_in(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    kind: WorkerKind,
    leaf_logics: &[LogicRef],
    combine_logic: LogicRef,
) -> Result<WorkflowDef, CompileError> {
    if leaf_logics.is_empty() {
        return Err(CompileError::EmptyRecipe);
    }
    let warrant = permissive_warrant(model_id.clone());
    let mut wf = WorkflowDef::new(seed);

    let mut leaves = Vec::with_capacity(leaf_logics.len());
    for logic in leaf_logics {
        let step = match kind {
            WorkerKind::Transform => transform(
                *logic,
                model_id.clone(),
                warrant.clone(),
                capability.clone(),
            ),
            WorkerKind::Generator => generator(
                *logic,
                model_id.clone(),
                warrant.clone(),
                capability.clone(),
            ),
        };
        leaves.push(wf.add_step(step));
    }
    let combine = wf.add_step(transform(combine_logic, model_id, warrant, capability));
    for &leaf in &leaves {
        wf.add_edge(leaf, combine, EdgeMeta::data())?;
    }
    Ok(wf)
}

/// **map-reduce.** `N` mapper steps (each [`WorkerKind`]) → one PURE reduce step
/// that consumes every mapper on a Data edge. Static `N` (`mapper_logics.len()`).
///
/// `WorkerKind::Transform` mappers are PURE (a deterministic map); `Generator`
/// mappers sample (a model map). The reduce is always a PURE `transform` — a
/// deterministic fold of the committed mapper outputs.
///
/// # Errors
///
/// [`CompileError::EmptyRecipe`] if `mapper_logics` is empty; propagates any
/// edge-declaration error from [`WorkflowDef::add_edge`](crate::WorkflowDef::add_edge).
pub fn map_reduce(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    kind: WorkerKind,
    mapper_logics: &[LogicRef],
    reduce_logic: LogicRef,
) -> Result<WorkflowDef, CompileError> {
    fan_in(
        seed,
        model_id,
        capability,
        kind,
        mapper_logics,
        reduce_logic,
    )
}

/// **fan-out / gather.** `N` parallel READ-ONLY-NONDET worker steps (independent
/// samplers) → one PURE gather step that folds all worker outputs. Static `N`
/// (`worker_logics.len()`).
///
/// Distinguished from [`map_reduce`] by intent: the workers here are always
/// non-deterministic generators (e.g. `N` independent model samples) and the
/// gather is a deterministic combine (e.g. majority vote, concatenation).
///
/// # Errors
///
/// [`CompileError::EmptyRecipe`] if `worker_logics` is empty; propagates edge errors.
pub fn fan_out_gather(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    worker_logics: &[LogicRef],
    gather_logic: LogicRef,
) -> Result<WorkflowDef, CompileError> {
    fan_in(
        seed,
        model_id,
        capability,
        WorkerKind::Generator,
        worker_logics,
        gather_logic,
    )
}

/// **RAG (retrieval-augmented generation).** A corpus of `doc_logics` — one
/// ingest/embed step per document (READ-ONLY-NONDET `generator`s: they read the
/// world/model to embed + populate a [`kx_dataset::RetrievalIndex`], cached by
/// ROND identity) → a single `retrieval` query step (ROND, the SN-8 boundary —
/// its committed fact is the ordered content refs, scores excluded) → a PURE
/// `assemble` step that grounds an answer in the retrieved top-k content
/// (consumed by exact hash). Static width (`doc_logics.len()`); `k` is the
/// authored retrieval width baked into the query step's logic.
///
/// Wiring: every `ingest_i ──data──> query` (so the query reads an index
/// populated from the committed ingest facts) and `query ──data──> assemble`
/// (the top-k fact is a Data-edge parent the assemble step reads via
/// `assemble()`). The execution glue that actually embeds + indexes + queries
/// lives in `kx-model-harness::rag` (it needs the FFI + dataset deps `kx-workflow`
/// must not carry); this builder is the pure-data structure.
///
/// # Errors
///
/// [`CompileError::EmptyRecipe`] if `doc_logics` is empty; propagates any edge
/// error from [`WorkflowDef::add_edge`](crate::WorkflowDef::add_edge).
pub fn rag_pipeline(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    doc_logics: &[LogicRef],
    query_logic: LogicRef,
    assemble_logic: LogicRef,
) -> Result<WorkflowDef, CompileError> {
    if doc_logics.is_empty() {
        return Err(CompileError::EmptyRecipe);
    }
    let warrant = permissive_warrant(model_id.clone());
    let mut wf = WorkflowDef::new(seed);

    // N ingest/embed steps — ROND (read the world/model, StageThenCommit, cached).
    let mut ingests = Vec::with_capacity(doc_logics.len());
    for logic in doc_logics {
        ingests.push(wf.add_step(generator(
            *logic,
            model_id.clone(),
            warrant.clone(),
            capability.clone(),
        )));
    }
    // The retrieval query step — ROND; the SN-8 boundary (similarity in, exact fact out).
    let query = wf.add_step(retrieval(
        query_logic,
        model_id.clone(),
        warrant.clone(),
        capability.clone(),
    ));
    // Each ingest feeds the query (its index is populated from the committed ingest facts).
    for &ingest in &ingests {
        wf.add_edge(ingest, query, EdgeMeta::data())?;
    }
    // The PURE assemble step — grounds an answer in the retrieved top-k (exact refs).
    let assemble = wf.add_step(transform(assemble_logic, model_id, warrant, capability));
    wf.add_edge(query, assemble, EdgeMeta::data())?;
    Ok(wf)
}

/// **Hybrid RAG (RC4c).** The retrieval-quality sibling of [`rag_pipeline`],
/// completing the authored RAG quartet:
///
/// ```text
/// rewrite (ROND) ─┐
///                 ├─data─> query[hybrid] (ROND) ─data─> [rerank (ROND)] ─data─> assemble (PURE)
/// ingest_i (ROND)─┘
/// ```
///
/// Three additions over [`rag_pipeline`]:
/// 1. a **`rewrite_query`** step the model uses to expand/rephrase the query before
///    retrieval (catches paraphrase gaps a single embedding misses);
/// 2. the `query` step bakes `config_subset[RETRIEVAL_MODE_KEY]="hybrid"` so its
///    `MoteId` is DISTINCT from the dense `rag_pipeline`'s query (a different
///    retrieval is a different fact — the dense recipe's golden `MoteId`s are
///    untouched), and the harness routes it to `query_corpus_hybrid` (BM25+dense
///    RRF + MMR) instead of `query_corpus`;
/// 3. an OPTIONAL **LLM listwise rerank** step (`rerank_logic = Some(_)`) between
///    `query` and `assemble` — a ROND turn whose model output is a grammar-constrained
///    permutation of the retrieved candidates (executed by `kx-model-harness::rerank_hits`,
///    fail-closed to the upstream order). `None` keeps the deterministic RRF+MMR order.
///
/// The execution glue (embed + dual-index + fuse + rerank) lives in
/// `kx-model-harness::rag`; this builder is the pure-data structure.
///
/// # Errors
///
/// [`CompileError::EmptyRecipe`] if `doc_logics` is empty; propagates any edge error
/// from [`WorkflowDef::add_edge`](crate::WorkflowDef::add_edge).
#[allow(clippy::too_many_arguments)] // the authored RAG quartet's logic refs are each meaningful
pub fn rag_pipeline_hybrid(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    doc_logics: &[LogicRef],
    rewrite_logic: LogicRef,
    query_logic: LogicRef,
    rerank_logic: Option<LogicRef>,
    assemble_logic: LogicRef,
) -> Result<WorkflowDef, CompileError> {
    if doc_logics.is_empty() {
        return Err(CompileError::EmptyRecipe);
    }
    let warrant = permissive_warrant(model_id.clone());
    let mut wf = WorkflowDef::new(seed);

    // N ingest/embed steps — ROND (read the world/model, StageThenCommit, cached).
    let mut ingests = Vec::with_capacity(doc_logics.len());
    for logic in doc_logics {
        ingests.push(wf.add_step(generator(
            *logic,
            model_id.clone(),
            warrant.clone(),
            capability.clone(),
        )));
    }
    // The query-rewrite step — ROND (committed: sampled once, served on replay).
    let rewrite = wf.add_step(rewrite_query(
        rewrite_logic,
        model_id.clone(),
        warrant.clone(),
        capability.clone(),
    ));
    // The HYBRID retrieval query step — ROND; the SN-8 boundary. The retrieval-mode
    // marker makes its MoteId distinct from the dense `rag_pipeline`'s query step.
    let mut query_step = retrieval(
        query_logic,
        model_id.clone(),
        warrant.clone(),
        capability.clone(),
    );
    query_step.config_subset.insert(
        ConfigKey(RETRIEVAL_MODE_KEY.to_string()),
        ConfigVal(b"hybrid".to_vec()),
    );
    let query = wf.add_step(query_step);
    // Each ingest feeds the query (its index is populated from the committed ingest facts).
    for &ingest in &ingests {
        wf.add_edge(ingest, query, EdgeMeta::data())?;
    }
    // The rewritten query feeds the retrieval step.
    wf.add_edge(rewrite, query, EdgeMeta::data())?;

    // Optional LLM listwise rerank between retrieval and assembly — a ROND model
    // turn (the harness `rerank_hits` dispatches it grammar-constrained; its distinct
    // `rerank_logic` keeps its MoteId separate from the rewrite/query steps).
    let grounded_by = if let Some(rerank_logic) = rerank_logic {
        let rerank = wf.add_step(generator(
            rerank_logic,
            model_id.clone(),
            warrant.clone(),
            capability.clone(),
        ));
        wf.add_edge(query, rerank, EdgeMeta::data())?;
        rerank
    } else {
        query
    };

    // The PURE assemble step — grounds an answer in the (re)ranked top-k (exact refs).
    let assemble = wf.add_step(transform(assemble_logic, model_id, warrant, capability));
    wf.add_edge(grounded_by, assemble, EdgeMeta::data())?;
    Ok(wf)
}

/// **retry-until-critic (bounded best-of-N).** `N` independent attempt steps
/// (READ-ONLY-NONDET generators), each gated by a [`deterministic_critic`] that
/// evaluates `check` against the attempt's committed bytes, and one PURE selector
/// that consumes every attempt + its verdict and picks the first that passes.
/// Static `N` (`attempt_logics.len()`) — the bound *is* the authored width.
///
/// The critic gates by **exact crypto-equality** of the [`CheckSpec`] outcome
/// (D60/D70); confidence may steer which attempt is preferred but never gates.
/// This is the single-level form of "retry until valid": a fixed budget of `N`
/// attempts judged in parallel. Unbounded sequential retry-until-pass is
/// iterative re-shaping (the cloud-tier topology); here a fresh budget is a fresh
/// appended round.
///
/// Wiring: `attempt_i ──data──> critic_i` (so each critic's `critic_for` resolves
/// to its attempt), and `attempt_i ──data──> select` + `critic_i ──data──> select`
/// (so the selector sees each candidate and its verdict). `critic_logic` and
/// `check` are reused across attempts — distinct producers yield distinct critic
/// identities.
///
/// # Errors
///
/// [`CompileError::EmptyRecipe`] if `attempt_logics` is empty; propagates edge /
/// critic-ordering errors.
pub fn retry_until_critic(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    attempt_logics: &[LogicRef],
    check: &CheckSpec,
    critic_logic: LogicRef,
    select_logic: LogicRef,
) -> Result<WorkflowDef, CompileError> {
    if attempt_logics.is_empty() {
        return Err(CompileError::EmptyRecipe);
    }
    let warrant = permissive_warrant(model_id.clone());
    let mut wf = WorkflowDef::new(seed);

    let mut attempts = Vec::with_capacity(attempt_logics.len());
    for logic in attempt_logics {
        attempts.push(wf.add_step(generator(
            *logic,
            model_id.clone(),
            warrant.clone(),
            capability.clone(),
        )));
    }
    let mut critics = Vec::with_capacity(attempts.len());
    for &attempt in &attempts {
        let critic = wf.add_step(deterministic_critic(
            attempt,
            check.clone(),
            critic_logic,
            model_id.clone(),
            warrant.clone(),
            capability.clone(),
        ));
        wf.add_edge(attempt, critic, EdgeMeta::data())?;
        critics.push(critic);
    }
    let select = wf.add_step(transform(select_logic, model_id, warrant, capability));
    for (&attempt, &critic) in attempts.iter().zip(critics.iter()) {
        wf.add_edge(attempt, select, EdgeMeta::data())?;
        wf.add_edge(critic, select, EdgeMeta::data())?;
    }
    Ok(wf)
}

/// **`ReAct` tool loop (single turn).** One reason → act → observe turn:
///
/// ```text
/// reason (ROND) ──data──> act (ROND, tool_contract) ──data──> observe (PURE)
/// ```
///
/// `reason` is the model's reasoning step; `act` calls a tool (its
/// `tool_contract` is the closed, pinned allowlist of tools it may invoke);
/// `observe` deterministically folds the tool result for the next turn. This is
/// the single-level "turn-batch": one authored iteration. A multi-turn `ReAct` agent is
/// expressed by **appending a fresh round** per turn (the planner's D76/D77
/// replanning, each round reading the prior turn's committed observation) — never
/// a static back-edge (a Mote's identity derives from its inputs, so a cycle is
/// unrepresentable) and never an in-workflow self-re-shaping loop (cloud tier).
///
/// # Errors
///
/// Propagates edge-declaration errors (the fixed three-step shape never produces one).
pub fn react_tool_loop(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    reason_logic: LogicRef,
    act_logic: LogicRef,
    observe_logic: LogicRef,
    tool_contract: BTreeMap<ToolName, ToolVersion>,
) -> Result<WorkflowDef, CompileError> {
    let warrant = permissive_warrant(model_id.clone());
    let mut wf = WorkflowDef::new(seed);

    let reason = wf.add_step(generator(
        reason_logic,
        model_id.clone(),
        warrant.clone(),
        capability.clone(),
    ));
    let mut act_step = generator(
        act_logic,
        model_id.clone(),
        warrant.clone(),
        capability.clone(),
    );
    act_step.tool_contract = tool_contract;
    let act = wf.add_step(act_step);
    let observe = wf.add_step(transform(observe_logic, model_id, warrant, capability));

    wf.add_edge(reason, act, EdgeMeta::data())?;
    wf.add_edge(act, observe, EdgeMeta::data())?;
    Ok(wf)
}

/// **image-batch describe-reduce (multi-modal capstone — authoring scaffold).**
/// One describe step per image (READ-ONLY-NONDET generators, all running the same
/// `describe_logic`) → one PURE reduce step that folds every description. Static
/// `N` (`image_refs.len()`).
///
/// Each describe step records *its own* image's content ref under
/// [`IMAGE_REF_KEY`], baked at authoring time — so the `N` describe steps describe
/// `N` **distinct** images and have distinct identities (the image ref folds into
/// `config_subset` → `MoteId`; a different image is a different Mote). This builds
/// the correct DAG shape + per-image identities; to actually *run* it
/// multi-modally the harness supplies each image as the describe step's
/// image-sniffed Data-edge **parent** content (the PR-2 dispatch path routes image
/// parents — not this `config_subset` key — as `content_ref`s) — the local
/// Gemma-4 milestone wiring. The reduce folds the committed descriptions into one
/// summary.
///
/// # Errors
///
/// [`CompileError::EmptyRecipe`] if `image_refs` is empty; propagates edge errors.
pub fn image_batch_describe_reduce(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    describe_logic: LogicRef,
    image_refs: &[ContentRef],
    reduce_logic: LogicRef,
) -> Result<WorkflowDef, CompileError> {
    if image_refs.is_empty() {
        return Err(CompileError::EmptyRecipe);
    }
    let warrant = permissive_warrant(model_id.clone());
    let mut wf = WorkflowDef::new(seed);

    let mut describes = Vec::with_capacity(image_refs.len());
    for image in image_refs {
        let mut step = generator(
            describe_logic,
            model_id.clone(),
            warrant.clone(),
            capability.clone(),
        );
        // Bake this step's image ref (identity-bearing) — distinct images yield
        // distinct describe identities.
        step.config_subset.insert(
            ConfigKey(IMAGE_REF_KEY.to_string()),
            ConfigVal(image.0.to_vec()),
        );
        describes.push(wf.add_step(step));
    }
    let reduce = wf.add_step(transform(reduce_logic, model_id, warrant, capability));
    for &d in &describes {
        wf.add_edge(d, reduce, EdgeMeta::data())?;
    }
    Ok(wf)
}
