//! [`compile`] — the pure, total, deterministic pass that turns a
//! [`WorkflowDef`] into a Mote DAG.
//!
//! **Determinism is the contract.** The same `WorkflowDef` produces
//! byte-identical `MoteId`s across runs, processes, and machines. The pass
//! touches no clock, host, PID, or allocation order: every map is a `BTreeMap`,
//! every traversal order is explicit (a min-index frontier in the topological
//! sort; parents canonically sorted by `MoteId`), and all identity bytes flow
//! from the workflow definition alone. This is what lets a compiled workflow be
//! a *reproducible program* — the basis of synthesis-by-recipe (D50).
//!
//! **Why a DAG.** A Mote's identity is `blake3(mote_def_hash ‖ input_data_id ‖
//! graph_position)`, and `input_data_id` derives from its parents — so a cycle
//! would make identity depend on its own output. Loops/branches/fan-out are not
//! static cycles; they are expressed at runtime via topology shapers that
//! materialize children deterministically (D23/D37).

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use kx_mote::{
    EdgeMeta, GraphPosition, InputDataId, Mote, MoteDef, MoteGraph, MoteId, ParentRef,
    MOTE_DEF_SCHEMA_VERSION,
};
use smallvec::SmallVec;

use crate::def::{CompiledMote, CompiledWorkflow, StepDef, StepRole, WorkflowDef};
use crate::error::CompileError;

/// Validated adjacency derived from a [`WorkflowDef`]'s edges.
struct Adjacency {
    /// In-degree per step index.
    indegree: Vec<usize>,
    /// `children[p]` = the step indices that depend on step `p`.
    children: Vec<Vec<usize>>,
    /// `parents_of[c]` = each parent of step `c` with its edge metadata.
    parents_of: Vec<Vec<(usize, EdgeMeta)>>,
}

/// Compile a [`WorkflowDef`] into a deterministic Mote DAG.
///
/// Returns the compiled motes in topological (submission) order plus a
/// [`MoteGraph`] view. Compilation is pure: identical input yields byte-
/// identical `MoteId`s.
///
/// # Errors
///
/// - [`CompileError::StepIndexOutOfRange`] — an edge or critic producer names a
///   non-existent step.
/// - [`CompileError::DuplicateEdge`] — the same `(parent, child)` edge twice.
/// - [`CompileError::Cycle`] — the declared edges are not acyclic.
/// - [`CompileError::InvalidCritic`] — a critic's producer does not precede it.
pub fn compile(def: &WorkflowDef) -> Result<CompiledWorkflow, CompileError> {
    let n = def.steps.len();
    let adj = build_adjacency(def, n)?;
    let order = topo_order(n, &adj)?;

    let mut mote_ids: Vec<Option<MoteId>> = vec![None; n];
    let mut motes: Vec<CompiledMote> = Vec::with_capacity(n);
    let mut graph = MoteGraph::new();

    for (rank, &step_idx) in order.iter().enumerate() {
        let s = &def.steps[step_idx];
        let critic_for = resolve_critic(s, step_idx, &mote_ids)?;
        let parents = resolve_parents(step_idx, &adj.parents_of, &mote_ids)?;

        // u64 is the fixed wire width for positions (usize would diverge across
        // 32/64-bit targets); u64::try_from over a usize never truncates.
        let rank_u64 = u64::try_from(rank).unwrap_or(u64::MAX);
        let graph_position = GraphPosition(rank_u64.to_le_bytes().to_vec());
        let input_data_id = if parents.is_empty() {
            entrypoint_input_id(def.seed(), rank_u64)
        } else {
            child_input_id(&parents)
        };

        let mote = Mote::new(
            mote_def_for(s, critic_for),
            input_data_id,
            graph_position,
            parents,
        );
        mote_ids[step_idx] = Some(mote.id);
        graph.insert(mote.clone());
        motes.push(CompiledMote {
            mote,
            warrant: s.warrant.clone(),
            capability: s.capability.clone(),
            step_index: step_idx,
        });
    }

    Ok(CompiledWorkflow { motes, graph })
}

/// Validate edges + critic producer ranges and build the adjacency.
fn build_adjacency(def: &WorkflowDef, n: usize) -> Result<Adjacency, CompileError> {
    let mut indegree = vec![0usize; n];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut parents_of: Vec<Vec<(usize, EdgeMeta)>> = vec![Vec::new(); n];
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();

    for e in &def.edges {
        let (p, c) = (e.parent.index(), e.child.index());
        if p >= n {
            return Err(CompileError::StepIndexOutOfRange(p));
        }
        if c >= n {
            return Err(CompileError::StepIndexOutOfRange(c));
        }
        if !seen.insert((p, c)) {
            return Err(CompileError::DuplicateEdge {
                parent: p,
                child: c,
            });
        }
        children[p].push(c);
        parents_of[c].push((p, e.edge));
        indegree[c] += 1;
    }

    for s in &def.steps {
        let producer = match &s.role {
            StepRole::Critic { producer } | StepRole::DeterministicCritic { producer, .. } => {
                Some(*producer)
            }
            _ => None,
        };
        if let Some(producer) = producer {
            if producer.index() >= n {
                return Err(CompileError::StepIndexOutOfRange(producer.index()));
            }
        }
    }

    Ok(Adjacency {
        indegree,
        children,
        parents_of,
    })
}

/// Kahn topological sort with a deterministic min-index frontier. Returns the
/// step indices in dependency order, or [`CompileError::Cycle`].
fn topo_order(n: usize, adj: &Adjacency) -> Result<Vec<usize>, CompileError> {
    let mut indegree = adj.indegree.clone();
    let mut frontier: BinaryHeap<Reverse<usize>> =
        (0..n).filter(|&i| indegree[i] == 0).map(Reverse).collect();
    let mut order: Vec<usize> = Vec::with_capacity(n);

    while let Some(Reverse(node)) = frontier.pop() {
        order.push(node);
        let mut cs = adj.children[node].clone();
        cs.sort_unstable(); // children released in ascending index order
        for c in cs {
            indegree[c] -= 1;
            if indegree[c] == 0 {
                frontier.push(Reverse(c));
            }
        }
    }

    if order.len() == n {
        Ok(order)
    } else {
        // Some node never reached in-degree zero → it sits on a cycle.
        let stuck = (0..n).find(|&i| indegree[i] > 0).unwrap_or(0);
        Err(CompileError::Cycle(stuck))
    }
}

/// Resolve a step's `critic_for`: `Some(producer MoteId)` for a critic whose
/// producer already compiled, else `None` for non-critics.
fn resolve_critic(
    s: &StepDef,
    step_idx: usize,
    mote_ids: &[Option<MoteId>],
) -> Result<Option<MoteId>, CompileError> {
    match &s.role {
        StepRole::Critic { producer } | StepRole::DeterministicCritic { producer, .. } => {
            match mote_ids[producer.index()] {
                Some(id) => Ok(Some(id)),
                None => Err(CompileError::InvalidCritic {
                    critic: step_idx,
                    producer: producer.index(),
                }),
            }
        }
        _ => Ok(None),
    }
}

/// Build a step's parent edges, canonically ordered by `MoteId`. Every parent
/// precedes the step in topological order, so its `MoteId` is already derived.
fn resolve_parents(
    step_idx: usize,
    parents_of: &[Vec<(usize, EdgeMeta)>],
    mote_ids: &[Option<MoteId>],
) -> Result<SmallVec<[ParentRef; 4]>, CompileError> {
    let mut parents: SmallVec<[ParentRef; 4]> = SmallVec::new();
    for (p, edge) in &parents_of[step_idx] {
        let Some(parent_id) = mote_ids[*p] else {
            // Unreachable under a valid topological order; treated as a
            // structural ordering defect.
            return Err(CompileError::Cycle(step_idx));
        };
        parents.push(ParentRef {
            parent_id,
            edge: *edge,
        });
    }
    parents.sort_unstable_by(|a, b| a.parent_id.0.cmp(&b.parent_id.0));
    Ok(parents)
}

/// Assemble the step's [`MoteDef`] at the current schema version.
fn mote_def_for(s: &StepDef, critic_for: Option<MoteId>) -> MoteDef {
    let critic_check = match &s.role {
        StepRole::DeterministicCritic { check, .. } => Some(check.clone()),
        _ => None,
    };
    MoteDef {
        critic_check,
        logic_ref: s.logic_ref,
        model_id: s.model_id.clone(),
        prompt_template_hash: s.prompt_template_hash,
        tool_contract: s.tool_contract.clone(),
        nd_class: s.nd_class,
        config_subset: s.config_subset.clone(),
        effect_pattern: s.effect_pattern,
        critic_for,
        is_topology_shaper: matches!(s.role, StepRole::TopologyShaper),
        inference_params: s.inference_params.clone(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

/// Deterministic `input_data_id` for an entrypoint (zero-parent) step: the
/// workflow seed folded with the step's topological rank. Per `kx-mote`, an
/// entrypoint's input id IS the BLAKE3 of a workflow-input seed — the workflow
/// SDK (this crate) is the blessed place to compute it.
fn entrypoint_input_id(seed: u32, rank: u64) -> InputDataId {
    let mut h = blake3::Hasher::new();
    h.update(b"kx-workflow/input-data-id/entrypoint/v1");
    h.update(&seed.to_le_bytes());
    h.update(&rank.to_le_bytes());
    InputDataId::from_bytes(*h.finalize().as_bytes())
}

/// Deterministic compile-time `input_data_id` for a step with parents, derived
/// from its (canonically sorted) parent lineage. The runtime executor owns the
/// authoritative derivation from committed parent `result_ref`s; this stable
/// compile-time value gives the workflow a reproducible identity ahead of any run.
fn child_input_id(parents: &[ParentRef]) -> InputDataId {
    let mut h = blake3::Hasher::new();
    h.update(b"kx-workflow/input-data-id/child/v1");
    for p in parents {
        h.update(p.parent_id.as_bytes());
        h.update(&[p.edge.kind.as_u8(), u8::from(p.edge.non_cascade)]);
    }
    InputDataId::from_bytes(*h.finalize().as_bytes())
}
