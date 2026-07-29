//! The ALWAYS-WIRED outer executor for deterministic workflow steps
//! (conditional / join-after-arms) — the steps that must run on EVERY build,
//! model or none ("deterministic steps never hold the model" is only true if
//! they never need the model executor to exist).
//!
//! It wraps whatever executor the serve built (the model router on a live
//! serve, the passthrough on a model-free one) and is the worker's single
//! [`ContextSink`]: every delivery is stashed for its OWN routes and TEED into
//! the inner sink (the model router's F-7 map) so both layers see the
//! identical parent context. Routed on identity-bearing config markers, the
//! `is_consensus_majority` posture — a mote without the marker runs exactly
//! as before.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_executor::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};
use kx_mote::{Mote, MoteId, NdClass};
use kx_warrant::{ExecutorClass, WarrantSpec};
use kx_worker::ContextSink;

/// Input cap for a conditional's source bytes (mirrors the model router's
/// critic-input cap — refuse, never truncate).
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

/// Bounded undelivered-context retention (the model router's KeyedSlots
/// posture, restated minimally: the live set is bounded by the lease batch).
const MAX_SLOTS: usize = 256;

fn internal(reason: &str) -> MoteExecutorError {
    MoteExecutorError::Internal {
        reason: reason.to_string(),
    }
}

/// One delivered context entry: the consumer Mote and its committed parents.
type SlotEntry = (MoteId, Vec<(MoteId, ContentRef)>);

/// A minimal insertion-ordered slot map (deliver-then-consume, evict-oldest).
#[derive(Default)]
struct Slots {
    entries: Mutex<VecDeque<SlotEntry>>,
}

impl Slots {
    fn set(&self, mote_id: MoteId, parents: Vec<(MoteId, ContentRef)>) {
        // A poisoned lock still holds a usable map (plain data, no invariants
        // spanning the panic) — recover it rather than propagate the poison.
        let mut e = match self.entries.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        e.retain(|(id, _)| *id != mote_id);
        e.push_back((mote_id, parents));
        while e.len() > MAX_SLOTS {
            e.pop_front();
        }
    }

    fn take(&self, mote_id: MoteId) -> Option<Vec<(MoteId, ContentRef)>> {
        let mut e = match self.entries.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let idx = e.iter().position(|(id, _)| *id == mote_id)?;
        e.remove(idx).map(|(_, v)| v)
    }
}

/// See the module docs.
pub(crate) struct DeterministicStepExecutor {
    inner: Arc<dyn MoteExecutor>,
    inner_sink: Option<Arc<dyn ContextSink>>,
    store: LocalFsContentStore,
    parent_ctx: Slots,
}

impl DeterministicStepExecutor {
    pub(crate) fn new(
        inner: Arc<dyn MoteExecutor>,
        store: LocalFsContentStore,
        inner_sink: Option<Arc<dyn ContextSink>>,
    ) -> Self {
        Self {
            inner,
            inner_sink,
            store,
            parent_ctx: Slots::default(),
        }
    }

    /// True iff `mote` is a workflow CONDITIONAL — a PURE step carrying the
    /// identity-bearing typed predicate.
    fn is_conditional(mote: &Mote) -> bool {
        mote.nd_class() == NdClass::Pure
            && mote
                .def
                .config_subset
                .contains_key(&kx_mote::ConfigKey(kx_mote::COND_PREDICATE_KEY.to_string()))
    }

    /// True iff `mote` is a workflow JOIN-after-arms (`first_non_skip`).
    fn is_join_select(mote: &Mote) -> bool {
        mote.nd_class() == NdClass::Pure
            && mote
                .def
                .config_subset
                .get(&kx_mote::ConfigKey(kx_mote::JOIN_SELECT_KEY.to_string()))
                .is_some_and(|v| v.0 == b"first_non_skip")
    }

    /// Evaluate the typed predicate over the step's SINGLE Data parent's
    /// committed bytes and commit the selection (`{"selected":"then"|"else"}`
    /// — canonical, byte-stable; a replay re-derives the identical decision).
    /// Fail-closed on a missing/plural parent, a malformed predicate, or
    /// unparseable JSON under a json-path op — a conditional that cannot
    /// decide dead-letters honestly and its arms stay unready (the run fails
    /// visibly, never half-branches).
    fn run_conditional(&self, mote: &Mote) -> Result<MoteExecutionResult, MoteExecutorError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Predicate {
            op: String,
            value: String,
            #[serde(default)]
            path: Option<String>,
            #[serde(default)]
            negate: bool,
        }
        let raw = mote
            .def
            .config_subset
            .get(&kx_mote::ConfigKey(kx_mote::COND_PREDICATE_KEY.to_string()))
            .ok_or_else(|| internal("conditional lost its predicate (config_subset)"))?;
        let p: Predicate = serde_json::from_slice(&raw.0)
            .map_err(|e| internal(&format!("conditional predicate malformed: {e}")))?;
        let parents = self.parent_ctx.take(mote.id).unwrap_or_default();
        let [(parent_id, parent_ref)] = parents.as_slice() else {
            return Err(internal(&format!(
                "conditional requires exactly ONE committed Data parent, got {}",
                parents.len()
            )));
        };
        let bytes = self
            .store
            .get(parent_ref)
            .map_err(|e| internal(&format!("read conditional source {parent_id:?}: {e}")))?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(internal(&format!(
                "conditional source {} bytes exceeds max {MAX_SOURCE_BYTES}",
                bytes.len()
            )));
        }
        let text = String::from_utf8_lossy(&bytes);
        let holds = match p.op.as_str() {
            "equals" => text == p.value,
            "contains" => text.contains(&p.value),
            // A dot-path walk over the parent's JSON ("$.a.b"); the addressed
            // scalar compares by its canonical string form. Unparseable JSON /
            // a missing path is fail-closed (never a silent `else`).
            "json_path_eq" => {
                let path = p
                    .path
                    .as_deref()
                    .ok_or_else(|| internal("json_path_eq requires a path (e.g. \"$.status\")"))?;
                let doc: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                    internal(&format!(
                        "conditional source is not JSON (json_path_eq): {e}"
                    ))
                })?;
                let mut cur = &doc;
                for seg in path
                    .trim_start_matches('$')
                    .split('.')
                    .filter(|s| !s.is_empty())
                {
                    cur = cur.get(seg).ok_or_else(|| {
                        internal(&format!("json path {path:?} missing segment {seg:?}"))
                    })?;
                }
                match cur {
                    serde_json::Value::String(s) => s == &p.value,
                    other => {
                        let canonical = other.to_string();
                        canonical == p.value
                    }
                }
            }
            other => {
                return Err(internal(&format!(
                    "conditional op must be equals|contains|json_path_eq, got {other:?}"
                )));
            }
        };
        let selected = if holds ^ p.negate { "then" } else { "else" };
        let out = format!("{{\"selected\":\"{selected}\"}}").into_bytes();
        let result_ref = self
            .store
            .put(&out)
            .map_err(|e| internal(&format!("content store put (conditional): {e}")))?;
        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }

    /// Commit the SINGLE non-SKIP parent's bytes VERBATIM (content-addressed —
    /// the same ref). 0 or 2+ survivors fail closed: a conditional that ran
    /// both arms (or neither) is a bug the join refuses to paper over.
    fn run_join_select(&self, mote: &Mote) -> Result<MoteExecutionResult, MoteExecutorError> {
        let skip_ref = ContentRef::of(kx_mote::COND_SKIP_SENTINEL);
        let parents = self.parent_ctx.take(mote.id).unwrap_or_default();
        if parents.is_empty() {
            return Err(internal("join(first_non_skip) has no committed parents"));
        }
        let survivors: Vec<&(MoteId, ContentRef)> =
            parents.iter().filter(|(_, r)| *r != skip_ref).collect();
        let [(winner_id, winner_ref)] = survivors.as_slice() else {
            return Err(internal(&format!(
                "join(first_non_skip) requires exactly ONE non-skip parent, got {} of {}",
                survivors.len(),
                parents.len()
            )));
        };
        let bytes = self
            .store
            .get(winner_ref)
            .map_err(|e| internal(&format!("read join survivor {winner_id:?}: {e}")))?;
        let result_ref = self
            .store
            .put(&bytes)
            .map_err(|e| internal(&format!("content store put (join): {e}")))?;
        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }
}

impl MoteExecutor for DeterministicStepExecutor {
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        if Self::is_conditional(mote) {
            return self.run_conditional(mote);
        }
        if Self::is_join_select(mote) {
            return self.run_join_select(mote);
        }
        self.inner.run(mote, warrant, env)
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        self.inner.supports(executor_class)
    }
}

impl ContextSink for DeterministicStepExecutor {
    fn set_parent_results(&self, mote_id: MoteId, parents: Vec<(MoteId, ContentRef)>) {
        // TEE: stash for our own routes AND forward so the inner (model
        // router's) F-7 map sees the identical delivery.
        self.parent_ctx.set(mote_id, parents.clone());
        if let Some(sink) = &self.inner_sink {
            sink.set_parent_results(mote_id, parents);
        }
    }

    fn set_context_items(&self, mote_id: MoteId, context_items_ref: Option<ContentRef>) {
        if let Some(sink) = &self.inner_sink {
            sink.set_context_items(mote_id, context_items_ref);
        }
    }

    fn set_image_ref(&self, mote_id: MoteId, image_ref: Option<ContentRef>) {
        if let Some(sink) = &self.inner_sink {
            sink.set_image_ref(mote_id, image_ref);
        }
    }
}
