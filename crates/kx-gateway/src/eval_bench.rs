//! The live-run oracle benchmark — fold a served-model ReAct run into a
//! [`kx_eval::Transcript`] and score it with the golden oracle scorers.
//!
//! This is the missing leg of the eval harness. The deterministic tier scores scripted
//! fixtures; the per-run `ScoreRun` RPC reads only the trajectory (turns/tools/terminal),
//! never the committed answer — so no served run was ever graded against a task's
//! `Expectation`. Here a REAL run is folded into a full transcript **client-side** from
//! the read RPCs (`ListReactTurns` + `GetProjection` + `GetContent`), then scored by the
//! same `score_transcript` the fixtures use. `kx-eval` stays a proto-free leaf (the fold
//! lives here, in the proto-aware gateway); `ScoreRun` stays byte-identical.
//!
//! ## Run isolation
//! A live serve shares ONE `instance_id` across every Invoke, so a run is NOT identified
//! by its `instance_id`. Each fold is scoped by the per-invocation chain key
//! (`InvokeResponse.react_chain_salt`, passed as `ListReactTurns.step_salt`) and reads its
//! answer from the invocation's server-derived `terminal_mote_id` — never a bare
//! instance-wide read, which would mix concurrent chains' turns.
//!
//! ## Coverage families
//! A bench task's `family` selects HOW it is driven ([`drive_for`]) — the substrate the
//! task is meant to exercise, not a label:
//!
//! | family  | shape                                    | fold                        |
//! |---------|------------------------------------------|-----------------------------|
//! | `tool`  | `Invoke` `react-auto`                    | [`fold_run_transcript`]     |
//! | `react` | `Invoke` `react-auto`                    | [`fold_run_transcript`]     |
//! | `script`| `Invoke` `react-auto`                    | [`fold_run_transcript`]     |
//! | `reach` | `Invoke` `react-rag`/`react-memory`, or `RunApp` | [`fold_run_transcript`] (+ observations) |
//! | `swarm` | `SubmitWorkflow`                         | [`fold_workflow_transcript`]|
//!
//! Every shape but the last settles a ReAct chain, so they share one fold. A swarm is a
//! plain multi-step DAG — it has NO ReAct turns and its `react_chain_salt` is empty by
//! design — so it is scoped by `RunHandle.terminal_mote_id` (the run anchor populated for
//! EVERY shape) and folded by walking that Mote's ancestors.
//!
//! An unknown family is a hard error, never a silent fall-through to the react path: a
//! task driven down the wrong shape still produces a number, and a wrong number that
//! looks right is worse than no number.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

use kx_eval::{
    aggregate, score_transcript, BenchCorpus, Branch, EvalReport, GoldenTask, ScoreInput,
    ScoreOutput, TaskScore, Transcript, TranscriptTiming, TurnRecord,
};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

use crate::provision::{
    REACT_AUTO_RECIPE_HANDLE, REACT_MEMORY_RECIPE_HANDLE, REACT_RAG_RECIPE_HANDLE,
};

/// The dataset a `reach` retrieval task searches. The benchmark driver ingests the
/// grounding corpus into it before scoring; the task instruction never names it (the
/// recipe folds the selector in advisorily).
pub const BENCH_DATASET: &str = "kx-bench";

/// The saved-App handle the `reach-inherit-principal` task runs. An App handle is an
/// `namespace/collection/name` AssetPath, not a bare name.
pub const BENCH_REACH_APP_HANDLE: &str = "kx/bench/reach";

/// The separator between a swarm task's participant prompts. The instruction of a
/// `swarm`-family task is a `---`-delimited list: every segment but the LAST is a
/// parallel agent, the last is the gather that reads all of their committed outputs.
const SWARM_PROMPT_SEP: &str = "\n---\n";

/// The admitted turn cap sent for every bench task (generous headroom over the ideal;
/// the transcript records the actual caps for the loop-efficiency scorer).
const BENCH_MAX_TURNS: u32 = 8;
/// The admitted tool-call cap sent for every bench task.
const BENCH_MAX_TOOL_CALLS: u32 = 6;

/// Why a live bench run could not be driven or folded.
#[derive(Debug)]
pub enum BenchError {
    /// An RPC failed (the failing method + the transport status).
    Rpc(&'static str, tonic::Status),
    /// The task instruction could not be encoded as the Invoke args JSON.
    EncodeArgs(serde_json::Error),
    /// The chain did not settle a terminal branch within the per-task timeout.
    NotSettled(String),
    /// The task's `family` (or a family task's id) names no known drive shape. Fails
    /// closed: driving it down the default react path would score a real number for a
    /// substrate the task never touched.
    UnknownFamily {
        /// The task that could not be dispatched.
        task_id: String,
        /// Its declared family.
        family: String,
    },
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchError::Rpc(method, status) => write!(f, "{method} rpc failed: {status}"),
            BenchError::EncodeArgs(e) => write!(f, "encode invoke args: {e}"),
            BenchError::NotSettled(id) => {
                write!(f, "task {id:?} did not settle a terminal branch in time")
            }
            BenchError::UnknownFamily { task_id, family } => write!(
                f,
                "task {task_id:?} declares family {family:?}, which no drive shape covers"
            ),
        }
    }
}

impl std::error::Error for BenchError {}

/// How a bench task is driven on a live serve — the shape its family names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drive {
    /// Invoke a ReAct recipe by handle, optionally naming a dataset to search.
    React {
        /// The recipe handle to invoke.
        handle: &'static str,
        /// The `dataset` selector folded into the instruction (react-rag only).
        dataset: Option<&'static str>,
    },
    /// Run a saved App (its own envelope decides the entry step's tool contract).
    App {
        /// The saved App handle to run.
        handle: &'static str,
    },
    /// Submit a multi-agent fan-out/gather chain and fold from the run's terminal Mote.
    Swarm,
}

impl Drive {
    /// The recipe handle this shape needs provisioned, if any. `None` ⇒ the shape needs
    /// no recipe (an App run or a raw workflow submission).
    #[must_use]
    pub fn required_recipe(&self) -> Option<&'static str> {
        match self {
            Drive::React { handle, .. } => Some(handle),
            // An App's entry step and a submitted workflow are bound from the request,
            // not resolved from the recipe catalog.
            Drive::App { .. } | Drive::Swarm => None,
        }
    }
}

/// Resolve a task's drive shape from its `family` (and, within `reach`, its id — the
/// three reach tasks exercise three genuinely different paths to the same property:
/// how far the runtime reaches beyond the prompt).
///
/// # Errors
/// [`BenchError::UnknownFamily`] for a family (or reach task id) with no drive shape.
// `Drive` is small while `BenchError` carries a `tonic::Status`, which trips
// `result_large_err` on this one function. Boxing the Status would ripple through every
// other `Result<_, BenchError>` in the module for one small Ok variant; one shared error
// type across the whole benchmark path is the clearer contract, so allow it here (the
// `kx-gateway-core::service::stream` precedent).
#[allow(clippy::result_large_err)]
pub fn drive_for(task: &GoldenTask) -> Result<Drive, BenchError> {
    let unknown = || BenchError::UnknownFamily {
        task_id: task.id.clone(),
        family: task.family.clone(),
    };
    match task.family.as_str() {
        // The tool-contract families both run the autogranted ReAct loop: one measures
        // that the granted tools FIRE, the other that an ungranted one does not.
        // `script` rides the same loop by design — a registered script IS a tool to
        // the model, and the family exists to measure that a SANDBOXED body actually
        // ran, not that a different calling convention works.
        //
        // The realism families ride it too, and for the same reason: what distinguishes
        // them is the TOOL they are given and the input they are handed, not a different
        // calling convention. `http` reaches a tool over the network under a credential;
        // `failure` is given tools that error, hang, or answer with garbage; `menu` has to
        // choose from a menu too long to read as a list of two; `long` has to hold a plan
        // across more turns than the loop was ever measured over; `adversarial` is handed
        // input that is trying to steer it. Driving them down a bespoke shape would test
        // the shape; driving them down the shape everything else uses is what makes their
        // numbers comparable with the rest of the table.
        // `irrelevance` rides the same loop again: the family's whole question is
        // whether the model, shown the SAME menu every tool family sees, declines to
        // fire when nothing on it applies — a bespoke drive would remove the menu the
        // decision is about.
        "tool" | "react" | "script" | "http" | "failure" | "menu" | "long" | "adversarial"
        | "irrelevance" => Ok(Drive::React {
            handle: REACT_AUTO_RECIPE_HANDLE,
            dataset: None,
        }),
        // The memory family runs the memory-tooled loop: both of its tasks are ABOUT
        // remember/recall, which only that recipe grants.
        "memory" => match task.id.as_str() {
            "memory-update-recall" | "memory-abstains-when-absent" => Ok(Drive::React {
                handle: REACT_MEMORY_RECIPE_HANDLE,
                dataset: None,
            }),
            _ => Err(unknown()),
        },
        "reach" => match task.id.as_str() {
            "rag-grounded-answer" => Ok(Drive::React {
                handle: REACT_RAG_RECIPE_HANDLE,
                dataset: Some(BENCH_DATASET),
            }),
            "memory-recall" => Ok(Drive::React {
                handle: REACT_MEMORY_RECIPE_HANDLE,
                dataset: None,
            }),
            "reach-inherit-principal" => Ok(Drive::App {
                handle: BENCH_REACH_APP_HANDLE,
            }),
            _ => Err(unknown()),
        },
        "swarm" => Ok(Drive::Swarm),
        _ => Err(unknown()),
    }
}

/// Fold ONE settled live run into a full [`Transcript`] — the trajectory from
/// `ListReactTurns` (scoped to this chain) plus the committed final answer from the run's
/// answer-branch turn.
///
/// `chain_salt` is `InvokeResponse.react_chain_salt` (empty ⇒ the legacy run-level chain).
/// The answer text is the committed content of the LAST `answer`-branch turn's Mote — NOT
/// the invocation's recipe sink (`terminal_mote_id`), whose committed value is a fold
/// wrapper, not the model's prose.
///
/// `observation_tools` names the tools whose committed OBSERVATIONS are collected into
/// `retrieved_docs` — the grounding the `groundedness` / `memory_quality` scorers read.
/// Empty ⇒ no observation fetch (the tool families need none, and every fetch is two more
/// RPCs). The observation is read from the CHILD of the tool turn's Mote, not the turn
/// Mote itself — a ReAct turn commits the model's tool-call PROPOSAL and the runtime
/// fires the call in a separate observation Mote parented to it.
///
/// # Errors
/// [`BenchError::Rpc`] if any read RPC fails.
pub async fn fold_run_transcript(
    client: &mut KxGatewayClient<Channel>,
    instance_id: Vec<u8>,
    chain_salt: Vec<u8>,
    task_id: String,
    observation_tools: &BTreeSet<String>,
) -> Result<Transcript, BenchError> {
    // Scope to THIS invocation's chain — never the shared instance_id alone.
    let step_salt = (!chain_salt.is_empty()).then_some(chain_salt);
    let listing = client
        .list_react_turns(proto::ListReactTurnsRequest {
            limit: None,
            instance_id: Some(instance_id.clone()),
            step_salt,
        })
        .await
        .map_err(|e| BenchError::Rpc("list_react_turns", e))?
        .into_inner();

    // Oldest → newest, so terminal_branch()/turn counting are correct (a ToolBatch turn
    // fans into N rows sharing a seq, ordered by call_index).
    let mut rows = listing.turns;
    rows.sort_by(|a, b| a.seq.cmp(&b.seq).then(a.call_index.cmp(&b.call_index)));

    let (max_turns, max_tool_calls) = rows
        .first()
        .map_or((0, 0), |r| (r.max_turns, r.max_tool_calls));
    let turns: Vec<TurnRecord> = rows
        .iter()
        .map(|r| TurnRecord {
            turn: r.turn,
            branch: branch_from_wire(&r.branch),
            tool_id: r.tool_id.clone(),
            tool_version: r.tool_version.clone(),
            call_index: r.call_index,
            rejection_reason: r.rejection_reason.clone(),
        })
        .collect();

    // The committed answer = the LAST answer-branch turn's Mote content
    // (GetProjection → that turn's turn_mote_id → result_ref → GetContent). A dead-lettered
    // run has no answer turn ⇒ None (the task_success scorer then reads the terminal
    // branch, which is not Answer).
    let answer_mote = rows
        .iter()
        .rev()
        .find(|r| r.branch == "answer")
        .map(|r| r.turn_mote_id.clone());
    let final_answer = match answer_mote {
        Some(mote) => fetch_committed_text(client, &instance_id, &mote).await?,
        None => None,
    };

    // The grounding the run actually saw: each named tool's committed OBSERVATION, in
    // fire order. This is what makes `groundedness` / `memory_quality` measurable on a
    // live run at all — with it empty they score N/A and a reach family is decorative.
    let mut retrieved_docs = Vec::new();
    if !observation_tools.is_empty() {
        for r in rows.iter().filter(|r| r.branch == "tool") {
            if observation_tools.contains(&r.tool_id) {
                if let Some(text) =
                    fetch_observation_text(client, &instance_id, &r.turn_mote_id).await?
                {
                    retrieved_docs.push(text);
                }
            }
        }
    }

    Ok(Transcript {
        task_id,
        turns,
        final_answer,
        retrieved_docs,
        rerank: None,
        max_turns,
        max_tool_calls,
        // Attached by the caller, which alone knows the telemetry floor this run began
        // above — the fold has no way to tell one task's execution exhaust from another's.
        timing: None,
    })
}

/// Fold ONE settled NON-ReAct run (a multi-step DAG — the `swarm` family) into a
/// [`Transcript`], scoped by the run's server-derived terminal Mote.
///
/// A swarm has no ReAct turns to list and its `react_chain_salt` is EMPTY by design (the
/// salt is populated only for exactly one tool-granted step), so neither the salt nor
/// `ListReactTurns` can scope it. `RunHandle.terminal_mote_id` is the anchor that IS
/// populated for every shape: from it the run's own sub-graph is the terminal's ancestor
/// closure, which is what this walks — identity and topology, never a Mote COUNT (a
/// count read off a shared journal is right by accident).
///
/// Each committed ancestor becomes one turn (a swarm leaf genuinely commits its own
/// answer), ordered by `committed_seq`, with the terminal last — so
/// `Transcript::terminal_branch()` is the gather's and `turns_used()` is the real
/// fan-out width plus one.
///
/// # Errors
/// [`BenchError::Rpc`] if a read RPC fails; [`BenchError::NotSettled`] if the terminal
/// Mote does not commit a result within `settle_timeout`.
pub async fn fold_workflow_transcript(
    client: &mut KxGatewayClient<Channel>,
    instance_id: Vec<u8>,
    terminal_mote_id: Vec<u8>,
    task_id: String,
    settle_timeout: Duration,
) -> Result<Transcript, BenchError> {
    let polls = (settle_timeout.as_millis() / 250).max(1);
    for _ in 0..polls {
        let view = client
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.clone(),
                at_seq: None,
            })
            .await
            .map_err(|e| BenchError::Rpc("get_projection", e))?
            .into_inner();

        let committed = proto::MoteSnapshotState::Committed as i32;
        let terminal = view.motes.iter().find(|m| m.mote_id == terminal_mote_id);
        let Some(result_ref) = terminal
            .filter(|m| m.state == committed)
            .and_then(|m| m.result_ref.clone())
        else {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        };

        // The run's own sub-graph: the terminal's transitive parents. Scoping by the
        // ancestor closure (not the whole projection) is what keeps a shared-journal
        // serve's other runs out of this transcript.
        let by_id: BTreeMap<&[u8], &proto::MoteSnapshot> = view
            .motes
            .iter()
            .map(|m| (m.mote_id.as_slice(), m))
            .collect();
        let mut seen: BTreeSet<&[u8]> = BTreeSet::new();
        let mut queue: VecDeque<&[u8]> = VecDeque::new();
        queue.push_back(terminal_mote_id.as_slice());
        seen.insert(terminal_mote_id.as_slice());
        let mut ancestors: Vec<&proto::MoteSnapshot> = Vec::new();
        while let Some(id) = queue.pop_front() {
            let Some(m) = by_id.get(id) else { continue };
            if id != terminal_mote_id.as_slice() {
                ancestors.push(m);
            }
            for p in &m.parents {
                if seen.insert(p.parent_id.as_slice()) {
                    queue.push_back(p.parent_id.as_slice());
                }
            }
        }
        // Commit order, so the transcript reads oldest → newest with the gather last.
        ancestors.retain(|m| m.state == committed);
        ancestors.sort_by_key(|m| m.committed_seq.unwrap_or(u64::MAX));

        // A swarm leaf commits its own answer — a settled fan-out has no tool branch and
        // no pending branch. One record per committed ancestor, the gather appended last.
        let answer_turn = |turn: usize| TurnRecord {
            turn: u32::try_from(turn).unwrap_or(u32::MAX),
            branch: Branch::Answer,
            tool_id: String::new(),
            tool_version: String::new(),
            call_index: 0,
            rejection_reason: String::new(),
        };
        let mut turns: Vec<TurnRecord> = (0..ancestors.len()).map(answer_turn).collect();
        turns.push(answer_turn(turns.len()));

        let content = client
            .get_content(proto::GetContentRequest {
                content_ref: result_ref,
                instance_id: instance_id.clone(),
            })
            .await
            .map_err(|e| BenchError::Rpc("get_content", e))?
            .into_inner();

        let used = u32::try_from(turns.len()).unwrap_or(u32::MAX);
        return Ok(Transcript {
            task_id,
            turns,
            final_answer: Some(String::from_utf8_lossy(&content.payload).into_owned()),
            retrieved_docs: Vec::new(),
            rerank: None,
            // A DAG has no ReAct budget; record what it actually used.
            max_turns: used,
            max_tool_calls: 0,
            // Attached by the caller (see `fold_run_transcript`).
            timing: None,
        });
    }
    Err(BenchError::NotSettled(task_id))
}

/// The result of driving a live suite: the scored report, plus what was NOT covered.
#[derive(Debug)]
pub struct LiveSuiteOutcome {
    /// The aggregate report over the tasks that ran.
    pub report: EvalReport,
    /// Families skipped because the recipe their shape needs is not provisioned on this
    /// serve (e.g. `react-rag` without `hnsw`), each with the reason.
    pub skipped: Vec<SkippedFamily>,
    /// The folded transcript behind every scored task, in suite order. A gate number
    /// says a task failed; only the trajectory says WHY (which tool it proposed, what
    /// the runtime refused, where it dead-lettered) — and a benchmark you cannot
    /// interrogate is one you end up guessing about.
    pub transcripts: Vec<Transcript>,
}

impl LiveSuiteOutcome {
    /// Whether every task in the corpus ran. **A partial run must never be captured as a
    /// baseline:** the committed baseline is keyed by `suite_digest`, so a capture that
    /// silently omitted a family would ratchet the whole corpus against a subset and
    /// read, forever after, as full coverage.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.skipped.is_empty()
    }
}

/// The `missing_recipe` recorded for a task held back by [`task_filter`] rather than by
/// an unprovisioned recipe — so a filtered run reads unmistakably as a diagnostic and can
/// never be mistaken for missing coverage.
pub const FILTERED_OUT: &str = "(held back by KX_BENCH_ONLY)";

/// The optional `KX_BENCH_ONLY` task-id allowlist (comma-separated), for attributing a
/// loop change to one arm without driving all sixteen tasks on a served model.
///
/// Unset / empty ⇒ `None` ⇒ the whole suite runs, byte-identically to before. Every task
/// it holds back is reported as SKIPPED, so `LiveSuiteOutcome::is_complete` is false and a
/// baseline capture is refused — the filter cannot be used to ratchet the corpus against a
/// subset.
fn task_filter() -> Option<BTreeSet<String>> {
    let raw = std::env::var("KX_BENCH_ONLY").ok()?;
    let ids: BTreeSet<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// One family the serve could not drive, and why.
#[derive(Debug, Clone)]
pub struct SkippedFamily {
    /// The family that did not run.
    pub family: String,
    /// The recipe handle that was missing.
    pub missing_recipe: String,
    /// The task ids that were skipped with it.
    pub task_ids: Vec<String>,
}

/// Drive a whole bench suite on a served model and score every task against its
/// `Expectation`. Each task is driven by the shape its family names ([`drive_for`]),
/// polled to a terminal (bounded by `settle_timeout`), folded, and scored;
/// `format_coverage` is N/A for a live suite (it measures the static parse corpus).
///
/// A family whose recipe is not provisioned is SKIPPED and reported in
/// [`LiveSuiteOutcome::skipped`] rather than failing the suite — a serve built without
/// `hnsw` genuinely cannot run the retrieval family, and scoring it 0 would slander the
/// model for a build choice. The caller must refuse to capture a baseline from an
/// incomplete outcome.
///
/// # Errors
/// The first task that fails to dispatch, invoke, settle, or fold aborts the suite with
/// its [`BenchError`]. A missing recipe is not an error (it is a skip); a task whose
/// family names no shape at all is.
pub async fn score_live_suite(
    client: &mut KxGatewayClient<Channel>,
    corpus: &BenchCorpus,
    env_label: String,
    git_sha: String,
    settle_timeout: Duration,
) -> Result<LiveSuiteOutcome, BenchError> {
    let provisioned: BTreeSet<String> = client
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .map_err(|e| BenchError::Rpc("list_recipes", e))?
        .into_inner()
        .recipes
        .into_iter()
        .map(|r| r.handle)
        .collect();

    let mut per_task: Vec<TaskScore> = Vec::with_capacity(corpus.suite.tasks.len());
    let mut transcripts: Vec<Transcript> = Vec::with_capacity(corpus.suite.tasks.len());
    let mut skipped: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let only = task_filter();
    for task in &corpus.suite.tasks {
        // A DIAGNOSTIC filter for attributing a loop change to one arm without paying for
        // the whole suite. A filtered-out task is recorded as SKIPPED, which is what makes
        // this safe: `is_complete()` goes false, so the caller's baseline-capture guard
        // refuses the run. Without that, a filter would be a way to ratchet the entire
        // corpus against a hand-picked subset — the exact failure that guard exists for.
        if let Some(only) = only.as_ref() {
            if !only.contains(&task.id) {
                skipped
                    .entry((task.family.clone(), FILTERED_OUT.to_string()))
                    .or_default()
                    .push(task.id.clone());
                continue;
            }
        }
        let drive = drive_for(task)?;
        if let Some(handle) = drive.required_recipe() {
            if !provisioned.contains(handle) {
                skipped
                    .entry((task.family.clone(), handle.to_string()))
                    .or_default()
                    .push(task.id.clone());
                continue;
            }
        }
        // The telemetry floor for THIS task, read before it is dispatched: everything
        // the sidecar joins above it is this task's cost and no other's.
        let since_seq = telemetry_high_water(client).await;
        let (mut transcript, terminal_mote) =
            run_and_fold(client, task, &drive, settle_timeout).await?;
        transcript.timing = fold_timing(client, since_seq, &terminal_mote).await;
        let scores = score_transcript(&ScoreInput {
            transcript: &transcript,
            expect: &task.expect,
        });
        per_task.push(TaskScore {
            task_id: task.id.clone(),
            family: task.family.clone(),
            scores,
        });
        transcripts.push(transcript);
    }

    let format_na = ScoreOutput::not_applicable("format_coverage", "N/A for a live suite");
    let mut spikes = latency_spikes(&transcripts);
    spikes.extend(token_spikes(&per_task, &transcripts));
    Ok(LiveSuiteOutcome {
        report: aggregate(
            corpus.suite.id.clone(),
            corpus.suite_digest.clone(),
            per_task,
            &format_na,
            &spikes,
            env_label,
            git_sha,
        ),
        skipped: skipped
            .into_iter()
            .map(|((family, missing_recipe), task_ids)| SkippedFamily {
                family,
                missing_recipe,
                task_ids,
            })
            .collect(),
        transcripts,
    })
}

/// The tools whose committed observations a task's oracle actually reads. A task that
/// grounds on retrieved evidence (`grounded_in`) or on recalled memory
/// (`memory_must_recall`) needs the observation text; every other task needs none, and
/// each fetch costs two more RPCs.
fn observation_tools_for(task: &GoldenTask) -> BTreeSet<String> {
    if task.expect.grounded_in.is_empty() && task.expect.memory_must_recall.is_empty() {
        return BTreeSet::new();
    }
    task.expect
        .expected_tools
        .iter()
        .map(|t| t.tool_id.clone())
        .collect()
}

/// The suite's absolute speed numbers, recorded and never gated.
///
/// These are the numbers a reader wants ("how long does a task take?") and precisely the
/// numbers that must not be a gate: they are dominated by the host's GPU and would fail
/// on a slower machine with no code change at all. `model_time_share` is the gated
/// companion — a ratio, so a uniformly slower host moves both its terms together.
///
/// Percentiles are nearest-rank on the sorted per-task totals. Tasks whose timing did not
/// land contribute to nothing here, and `measured_tasks` says how many did — a p95 over
/// three of sixteen tasks is not a suite number, and the count is what makes that visible
/// instead of leaving a confident-looking figure to be read as full coverage.
fn latency_spikes(transcripts: &[Transcript]) -> Vec<ScoreOutput> {
    let mut totals: Vec<u64> = transcripts
        .iter()
        .filter_map(|t| t.timing)
        .map(|t| t.total_ms)
        .collect();
    let measured = totals.len();
    let mut out = vec![ScoreOutput {
        metric_id: "measured_tasks".to_string(),
        value: kx_eval::ScoreValue::Spike {
            #[allow(clippy::cast_precision_loss)]
            value: measured as f64,
            unit: "tasks".to_string(),
        },
        applicable: true,
        detail: format!(
            "{measured} of {} task(s) reported host timing",
            transcripts.len()
        ),
    }];
    if totals.is_empty() {
        return out;
    }
    totals.sort_unstable();
    let pct = |p: usize| -> u64 {
        // Nearest-rank: the smallest value at or above the p-th percentile.
        let rank = (p * totals.len()).div_ceil(100).max(1);
        totals[rank - 1]
    };
    let sum: u64 = totals.iter().sum();
    for (id, value) in [
        ("task_latency_ms_p50", pct(50)),
        ("task_latency_ms_p95", pct(95)),
        ("task_latency_ms_max", *totals.last().unwrap_or(&0)),
        ("suite_latency_ms_total", sum),
    ] {
        out.push(ScoreOutput {
            metric_id: id.to_string(),
            value: kx_eval::ScoreValue::Spike {
                #[allow(clippy::cast_precision_loss)]
                value: value as f64,
                unit: "ms".to_string(),
            },
            applicable: true,
            detail: format!("over {measured} measured task(s)"),
        });
    }
    out
}

/// An integer-valued Spike (tokens, counts) — the emission point is where rounding
/// happens, so a committed baseline diffs in whole units.
fn int_spike(id: &str, value: u64, unit: &str, detail: String) -> ScoreOutput {
    ScoreOutput {
        metric_id: id.to_string(),
        value: kx_eval::ScoreValue::Spike {
            #[allow(clippy::cast_precision_loss)]
            value: value as f64,
            unit: unit.to_string(),
        },
        applicable: true,
        detail,
    }
}

/// The suite's output-token economy, recorded and never gated.
///
/// Computed from the same telemetry windows the wall-clock split reads, so a task's
/// tokens are attributed exactly as its time is. `per_task[i]` and `transcripts[i]` are
/// pushed together by the driver loop, which is what lets a token sum sit beside its
/// task's family and pass/fail without a join.
///
/// - `tokens_per_task_mean` (suite and per family): integer mean over the tasks that
///   reported a count. `tokens_measured_tasks` is the coverage denominator beside them.
/// - `tokens_per_success`: total measured output tokens per PASSED measured task — the
///   cost of a success with the cost of the failures amortised into it. OMITTED when no
///   measured task passed: a cost-per-success with zero successes is not a big number,
///   it is no number (and its absence is what a reader should see).
///
/// There are no input-token spikes because OSS records no input count — a metric whose
/// input the runtime does not record is not published, ever.
fn token_spikes(per_task: &[TaskScore], transcripts: &[Transcript]) -> Vec<ScoreOutput> {
    debug_assert_eq!(per_task.len(), transcripts.len());
    let measured: Vec<(usize, u64)> = transcripts
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.timing.and_then(|tm| tm.output_tokens).map(|tok| (i, tok)))
        .collect();
    let mut out = vec![int_spike(
        "tokens_measured_tasks",
        measured.len() as u64,
        "tasks",
        format!(
            "{} of {} task(s) reported output-token counts",
            measured.len(),
            transcripts.len()
        ),
    )];
    if measured.is_empty() {
        return out;
    }
    let total: u64 = measured.iter().map(|(_, tok)| tok).sum();
    out.push(int_spike(
        "tokens_per_task_mean",
        total / measured.len() as u64,
        "tokens",
        format!("mean over {} measured task(s)", measured.len()),
    ));
    // Per family, in first-appearance order (the published table's order).
    let mut family_order: Vec<&str> = Vec::new();
    let mut family_tokens: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    for (i, tok) in &measured {
        let family = per_task[*i].family.as_str();
        if !family.is_empty() {
            if !family_order.contains(&family) {
                family_order.push(family);
            }
            family_tokens.entry(family).or_default().push(*tok);
        }
    }
    for family in family_order {
        let toks = &family_tokens[family];
        out.push(int_spike(
            &format!("tokens_per_task_mean@{family}"),
            toks.iter().sum::<u64>() / toks.len() as u64,
            "tokens",
            format!("mean over {} measured task(s)", toks.len()),
        ));
    }
    let passes = measured
        .iter()
        .filter(|(i, _)| {
            per_task[*i]
                .scores
                .iter()
                .any(|s| s.metric_id == "task_success" && s.gate_per_mille() == Some(1000))
        })
        .count() as u64;
    if passes > 0 {
        out.push(int_spike(
            "tokens_per_success",
            total / passes,
            "tokens",
            format!("total measured output tokens over {passes} passed task(s)"),
        ));
    }
    out
}

/// How long the harness waits for the telemetry sidecar's join tick to catch up with a
/// settled run, and at what cadence. The tick is driven by a journal-commit signal, so
/// the rows for a run's last motes land shortly AFTER the answer branch the settle poll
/// returned on — mirroring the answer-commit retry a few functions down.
const TELEMETRY_JOIN_POLLS: u32 = 50;
const TELEMETRY_JOIN_INTERVAL: Duration = Duration::from_millis(100);

/// The telemetry high-water mark — the newest joined row's journal seq, or 0 when the
/// sidecar has nothing (or does not exist).
///
/// Read from the same RPC the window fold reads, deliberately: a high-water taken from a
/// different source could disagree with the rows, and a window that disagrees with its
/// own bounds silently mis-attributes one task's cost to another.
async fn telemetry_high_water(client: &mut KxGatewayClient<Channel>) -> u64 {
    client
        .list_mote_telemetry(proto::ListMoteTelemetryRequest {
            limit: Some(1),
            instance_id: None,
            mote_id: None,
            before_seq: None,
        })
        .await
        .ok()
        .and_then(|r| r.into_inner().rows.first().map(|row| row.seq))
        .unwrap_or(0)
}

/// Fold the host's execution exhaust for ONE task into a [`TranscriptTiming`].
///
/// **Why a seq WINDOW and not a mote-id set.** A run's cost is spread over three kinds of
/// Mote — the model turns, the tool dispatches, and the observations parented to them —
/// and only the first kind is in `ListReactTurns`. Enumerating ids would silently omit
/// the other two and report a model share that is too high, i.e. it would flatter exactly
/// the thing being gated. The suite runs `--test-threads=1` and drives one task at a
/// time, so the journal seqs a task produced are precisely those above the high-water
/// read before it was dispatched.
///
/// Returns `None` — never a zero — when the sidecar is absent, when the window is empty,
/// or when `terminal_mote` never appears in it within the join budget. A partial window
/// would under-count the model's time and read as runtime overhead that never happened,
/// so a measurement that did not fully land is reported as no measurement at all.
async fn fold_timing(
    client: &mut KxGatewayClient<Channel>,
    since_seq: u64,
    terminal_mote: &[u8],
) -> Option<TranscriptTiming> {
    for attempt in 0..TELEMETRY_JOIN_POLLS {
        if attempt > 0 {
            tokio::time::sleep(TELEMETRY_JOIN_INTERVAL).await;
        }
        let rows = telemetry_window(client, since_seq).await;
        // The join has caught up exactly when the run's LAST Mote is in the window.
        // Row count is not a stopping condition: it can plateau mid-drain and would
        // stop early on a run whose tail is still folding.
        if !terminal_mote.is_empty() && !rows.iter().any(|r| r.mote_id == terminal_mote) {
            continue;
        }
        return timing_from_rows(&rows);
    }
    None
}

/// Every joined telemetry row with `seq > since_seq`, newest-first paging until the
/// window is exhausted. `ListMoteTelemetry` clamps its page to 500, so a long-horizon
/// task needs more than one page — reading only the first would truncate the window and
/// under-report the run's own cost.
async fn telemetry_window(
    client: &mut KxGatewayClient<Channel>,
    since_seq: u64,
) -> Vec<proto::MoteTelemetryRow> {
    let mut out: Vec<proto::MoteTelemetryRow> = Vec::new();
    let mut before: Option<u64> = None;
    loop {
        let Ok(resp) = client
            .list_mote_telemetry(proto::ListMoteTelemetryRequest {
                limit: Some(500),
                instance_id: None,
                mote_id: None,
                before_seq: before,
            })
            .await
        else {
            // A serve without the sidecar answers `unimplemented`. No rows, and the
            // caller turns that into "no timing" rather than a zero.
            return Vec::new();
        };
        let resp = resp.into_inner();
        if resp.rows.is_empty() {
            return out;
        }
        let oldest = resp.rows.iter().map(|r| r.seq).min().unwrap_or(0);
        let reached_floor = oldest <= since_seq;
        out.extend(resp.rows.into_iter().filter(|r| r.seq > since_seq));
        if reached_floor || !resp.has_more {
            return out;
        }
        before = Some(oldest);
    }
}

/// Split a task's telemetry window into the model's time and the runtime's.
///
/// `total_ms` is the span from the first Mote starting to the last one finishing — the
/// runtime's own end-to-end cost for the task, which deliberately excludes the harness's
/// fold RPCs (they are the benchmark's cost, not the runtime's, and charging them to the
/// runtime would make the gate measure the instrument). The gaps BETWEEN motes are inside
/// it, and on a multi-turn loop those gaps — scheduling, folding, committing, leasing —
/// are the overhead worth watching.
fn timing_from_rows(rows: &[proto::MoteTelemetryRow]) -> Option<TranscriptTiming> {
    if rows.is_empty() {
        return None;
    }
    let start = rows.iter().map(|r| r.started_unix_ms).min()?;
    let end = rows
        .iter()
        .map(|r| r.started_unix_ms.saturating_add(r.wall_clock_ms))
        .max()?;
    // A model Mote carries the model that ran it. Tool time is NOT split out: see
    // `TranscriptTiming` — the row's tool id comes from the declared contract, not from
    // what fired, so any tool total built from it would be a number that cannot move.
    let model_ms = rows
        .iter()
        .filter(|r| !r.model_id.is_empty())
        .map(|r| r.wall_clock_ms)
        .sum();
    // The same model rows carry the run's output-token counts. Summed here because the
    // window is already the task's exact cost attribution; `None` when no model row
    // reported a count (a degraded build) — absent is not zero.
    let token_rows: Vec<u64> = rows
        .iter()
        .filter(|r| !r.model_id.is_empty())
        .filter_map(|r| r.output_tokens)
        .collect();
    let output_tokens = (!token_rows.is_empty()).then(|| token_rows.iter().sum());
    Some(TranscriptTiming {
        total_ms: end.saturating_sub(start),
        model_ms,
        output_tokens,
    })
}

/// Drive ONE task through the shape its family names, and fold the settled run.
///
/// Returns the transcript beside the run's terminal Mote id (see
/// [`settle_and_fold_react`] — the timing fold waits on it).
async fn run_and_fold(
    client: &mut KxGatewayClient<Channel>,
    task: &GoldenTask,
    drive: &Drive,
    settle_timeout: Duration,
) -> Result<(Transcript, Vec<u8>), BenchError> {
    match drive {
        Drive::React { handle, dataset } => {
            // A task may raise its own budget. The suite default is sized for a two-hop
            // lookup; a long-horizon chain needs more, and running everything at the
            // long task's budget would stop any short task from ever reaching its cap —
            // which is itself something the suite measures.
            let mut args = serde_json::json!({
                "instruction": task.instruction,
                "max_turns": task.expect.max_turns.unwrap_or(BENCH_MAX_TURNS),
                "max_tool_calls": task.expect.max_tool_calls.unwrap_or(BENCH_MAX_TOOL_CALLS),
            });
            if let (Some(ds), Some(obj)) = (dataset, args.as_object_mut()) {
                obj.insert("dataset".into(), serde_json::Value::String((*ds).into()));
            }
            let args = serde_json::to_vec(&args).map_err(BenchError::EncodeArgs)?;
            let resp = client
                .invoke(proto::InvokeRequest {
                    handle: (*handle).to_string(),
                    args,
                    context_bundles: vec![],
                    context_refs: vec![],
                })
                .await
                .map_err(|e| BenchError::Rpc("invoke", e))?
                .into_inner();
            settle_and_fold_react(
                client,
                resp.instance_id,
                resp.react_chain_salt,
                task,
                settle_timeout,
            )
            .await
        }
        Drive::App { handle } => {
            // A saved App run is still a ReAct chain — its envelope has exactly one
            // agentic step, so the server populates the chain salt and the same fold
            // applies. What differs is WHERE the step's tool contract came from.
            let args =
                serde_json::to_vec(&serde_json::json!({})).map_err(BenchError::EncodeArgs)?;
            let run = client
                .run_app(proto::RunAppRequest {
                    handle: (*handle).to_string(),
                    args,
                    require_approval: false,
                })
                .await
                .map_err(|e| BenchError::Rpc("run_app", e))?
                .into_inner();
            settle_and_fold_react(
                client,
                run.instance_id,
                run.react_chain_salt,
                task,
                settle_timeout,
            )
            .await
        }
        Drive::Swarm => {
            let handle = client
                .submit_workflow(swarm_request(&task.instruction))
                .await
                .map_err(|e| BenchError::Rpc("submit_workflow", e))?
                .into_inner();
            let terminal_mote = handle.terminal_mote_id.clone();
            let transcript = fold_workflow_transcript(
                client,
                handle.instance_id,
                handle.terminal_mote_id,
                task.id.clone(),
                settle_timeout,
            )
            .await?;
            Ok((transcript, terminal_mote))
        }
    }
}

/// Poll a ReAct chain to a terminal branch, then fold it.
///
/// Returns the transcript beside the terminal turn's Mote id — the last Mote the run
/// executes, which the timing fold waits on to know the telemetry join has caught up.
async fn settle_and_fold_react(
    client: &mut KxGatewayClient<Channel>,
    instance_id: Vec<u8>,
    chain_salt: Vec<u8>,
    task: &GoldenTask,
    settle_timeout: Duration,
) -> Result<(Transcript, Vec<u8>), BenchError> {
    let step_salt = (!chain_salt.is_empty()).then(|| chain_salt.clone());
    let polls = (settle_timeout.as_millis() / 100).max(1);
    let mut terminal_mote: Option<Vec<u8>> = None;
    for _ in 0..polls {
        let t = client
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(instance_id.clone()),
                step_salt: step_salt.clone(),
            })
            .await
            .map_err(|e| BenchError::Rpc("list_react_turns", e))?
            .into_inner();
        if let Some(row) = t
            .turns
            .iter()
            .filter(|x| x.branch == "answer" || x.branch == "dead_lettered")
            .max_by_key(|x| x.seq)
        {
            terminal_mote = Some(row.turn_mote_id.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Some(terminal_mote) = terminal_mote else {
        return Err(BenchError::NotSettled(task.id.clone()));
    };

    let transcript = fold_run_transcript(
        client,
        instance_id,
        chain_salt,
        task.id.clone(),
        &observation_tools_for(task),
    )
    .await?;
    Ok((transcript, terminal_mote))
}

/// Lower a `swarm`-family instruction into the fan-out/gather chain `kx swarm` authors:
/// N parallel MODEL leaves (no inter-edges) fanned by Data edges into ONE MODEL gather
/// that reads every leaf's committed output. The instruction is `---`-separated — the
/// last segment is the gather, the rest are the agents.
///
/// The lowering is TOTAL: a single-segment instruction lowers to that one step with no
/// edges. It is the CORPUS that must declare a real fan-out (pinned by
/// `a_swarm_task_declares_a_real_fanout`) — silently duplicating the lone prompt into a
/// synthetic leaf would manufacture a fan-in the task never asked for, and the swarm gate
/// would pass on a chain that never fanned out.
fn swarm_request(instruction: &str) -> proto::SubmitWorkflowRequest {
    let model = |prompt: &str| proto::WorkflowStep {
        kind: proto::WorkflowStepKind::Model as i32,
        model_id: String::new(),
        prompt: prompt.to_string(),
        body_signature_id: Vec::new(),
        tool_contract: std::collections::HashMap::new(),
        params: std::collections::HashMap::new(),
    };
    let mut segments: Vec<&str> = instruction.split(SWARM_PROMPT_SEP).collect();
    let gather = segments.pop().unwrap_or(instruction);
    let steps: Vec<proto::WorkflowStep> = segments
        .iter()
        .map(|s| model(s))
        .chain(std::iter::once(model(gather)))
        .collect();
    let gather_index = u32::try_from(steps.len().saturating_sub(1)).unwrap_or(0);
    let edges = (0..gather_index)
        .map(|parent| proto::WorkflowEdge {
            parent,
            child: gather_index,
            edge_kind: proto::EdgeKind::Data as i32,
            non_cascade: false,
        })
        .collect();
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps,
        edges,
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: vec![],
    }
}

/// Fetch a committed Mote's result text, retrying briefly for the commit to land after the
/// branch settles. `None` when the Mote never commits a result.
async fn fetch_committed_text(
    client: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    mote_id: &[u8],
) -> Result<Option<String>, BenchError> {
    // Up to ~5s for the result_ref to commit after the answer branch appears.
    for _ in 0..50 {
        let view = client
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.to_vec(),
                at_seq: None,
            })
            .await
            .map_err(|e| BenchError::Rpc("get_projection", e))?
            .into_inner();
        if let Some(result_ref) = view
            .motes
            .iter()
            .find(|m| m.mote_id == mote_id)
            .and_then(|m| m.result_ref.clone())
        {
            let content = client
                .get_content(proto::GetContentRequest {
                    content_ref: result_ref,
                    instance_id: instance_id.to_vec(),
                })
                .await
                .map_err(|e| BenchError::Rpc("get_content", e))?
                .into_inner();
            return Ok(Some(String::from_utf8_lossy(&content.payload).into_owned()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(None)
}

/// Fetch the committed OBSERVATION text produced by one tool turn.
///
/// The observation is NOT the turn Mote's own content: a ReAct turn Mote commits the
/// model's tool-call PROPOSAL, and the runtime fires the call in a separate observation
/// Mote whose single parent is that turn (`kx_model_harness::workflows::react_tool_mote`).
/// Reading the turn Mote here would put the model's own request into `retrieved_docs` and
/// score groundedness against the question instead of the evidence — the same
/// wrong-Mote class as folding an answer from the invoke sink.
///
/// `None` when the observation has not committed (a fail-closed dispatch) — the scorer
/// then sees one fewer grounding doc, which is the honest reading.
async fn fetch_observation_text(
    client: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    turn_mote_id: &[u8],
) -> Result<Option<String>, BenchError> {
    let view = client
        .get_projection(proto::GetProjectionRequest {
            instance_id: instance_id.to_vec(),
            at_seq: None,
        })
        .await
        .map_err(|e| BenchError::Rpc("get_projection", e))?
        .into_inner();
    let observation = view
        .motes
        .iter()
        .filter(|m| m.parents.iter().any(|p| p.parent_id == turn_mote_id))
        .find_map(|m| m.result_ref.clone());
    let Some(result_ref) = observation else {
        return Ok(None);
    };
    let content = client
        .get_content(proto::GetContentRequest {
            content_ref: result_ref,
            instance_id: instance_id.to_vec(),
        })
        .await
        .map_err(|e| BenchError::Rpc("get_content", e))?
        .into_inner();
    Ok(Some(String::from_utf8_lossy(&content.payload).into_owned()))
}

/// The wire branch string emitted by `ListReactTurns` → the eval [`Branch`] (mirrors the
/// per-run scorer's mapping; unknown/pending strings fold to `Pending`).
fn branch_from_wire(s: &str) -> Branch {
    match s {
        "answer" => Branch::Answer,
        "tool" => Branch::Tool,
        "rejected" => Branch::Rejected,
        "dead_lettered" => Branch::DeadLettered,
        _ => Branch::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task's declared budget must be one the recipe will actually ADMIT.
    ///
    /// This cost a full 26-task run to discover. A task asked for 14 turns; the react
    /// recipe's parameter contract admits `1..=8` and re-validates it fail-closed, so the
    /// invoke was refused with `OutOfRange` — fifty minutes in, after every task before it
    /// had already been driven on the model. Nothing model-free knew the corpus could
    /// declare a budget the runtime would reject.
    ///
    /// The ceiling is not negotiable from here: it lives in the recipe body, and changing
    /// a recipe body under an unchanged id is refused as an immutability conflict at boot.
    /// So the corpus is what has to fit, and this is where that is enforced — in a test
    /// that runs in a second, rather than in a benchmark that runs for an hour.
    #[test]
    fn no_task_declares_a_budget_the_recipe_would_refuse() {
        // Mirrors `provision`'s react free-param contract: `0 < max_turns <= 8` and
        // `0 < max_tool_calls <= 20`, re-validated fail-closed on every invoke.
        const ADMITTED_MAX_TURNS: u32 = 8;
        const ADMITTED_MAX_TOOL_CALLS: u32 = 20;
        let corpus = kx_eval::load_bench_v1().expect("bench-v1 corpus loads");
        for t in &corpus.suite.tasks {
            if let Some(turns) = t.expect.max_turns {
                assert!(
                    turns > 0 && turns <= ADMITTED_MAX_TURNS,
                    "task {} declares max_turns {turns}, outside the admitted 1..={ADMITTED_MAX_TURNS}",
                    t.id
                );
            }
            if let Some(calls) = t.expect.max_tool_calls {
                assert!(
                    calls > 0 && calls <= ADMITTED_MAX_TOOL_CALLS,
                    "task {} declares max_tool_calls {calls}, outside the admitted \
                     1..={ADMITTED_MAX_TOOL_CALLS}",
                    t.id
                );
            }
            // And the ideal must be reachable inside the budget the task will run under,
            // or the task is unsatisfiable by construction and its score says nothing.
            let budget_turns = t.expect.max_turns.unwrap_or(BENCH_MAX_TURNS);
            assert!(
                t.expect.ideal_turns <= budget_turns,
                "task {} needs {} turns ideally but will be admitted only {budget_turns}",
                t.id,
                t.expect.ideal_turns
            );
            let budget_calls = t.expect.max_tool_calls.unwrap_or(BENCH_MAX_TOOL_CALLS);
            assert!(
                t.expect.ideal_tool_calls <= budget_calls,
                "task {} needs {} tool calls ideally but will be admitted only {budget_calls}",
                t.id,
                t.expect.ideal_tool_calls
            );
        }
    }

    /// One joined telemetry row. `model_id` and `tool_id` are the discriminator the
    /// split relies on: the sidecar upserts them from two different events, so a model
    /// Mote carries the first and a tool-bearing Mote the second.
    fn row(seq: u64, started: u64, wall: u64, model: &str, tool: &str) -> proto::MoteTelemetryRow {
        proto::MoteTelemetryRow {
            mote_id: vec![u8::try_from(seq % 256).unwrap_or(0); 32],
            instance_id: vec![7; 16],
            wall_clock_ms: wall,
            input_tokens: None,
            output_tokens: None,
            model_id: model.to_string(),
            tool_id: tool.to_string(),
            started_unix_ms: started,
            seq,
        }
    }

    /// The split the gate is built on: model time and tool time come off disjoint rows,
    /// and the wall-clock span covers the whole run including the gaps between motes.
    /// Those gaps are the runtime's own cost — scheduling, folding, committing — and if
    /// they were excluded the gate would have nothing left to detect.
    #[test]
    fn timing_attributes_model_and_tool_time_separately() {
        // t=0 model runs 100ms; a 50ms gap; t=150 a tool runs 30ms; another 20ms gap;
        // t=200 the answering model turn runs 60ms. Span 0..260.
        let rows = [
            row(10, 0, 100, "gemma", ""),
            row(11, 150, 30, "", "mcp-kv/get"),
            row(12, 200, 60, "gemma", ""),
        ];
        let t = timing_from_rows(&rows).expect("rows produce a timing");
        assert_eq!(t.total_ms, 260, "span from first start to last finish");
        assert_eq!(t.model_ms, 160, "both model motes, and only those");
        assert_eq!(
            t.total_ms - t.model_ms,
            100,
            "the 70ms of gaps plus the 30ms tool round are not the model's time"
        );
    }

    /// An empty window is NOT a zero-cost run — it is a run whose cost was not measured.
    /// A zero here would score `model_time_share` 0 and read as the runtime having
    /// consumed the entire task, which is a fabricated regression.
    #[test]
    fn an_empty_telemetry_window_is_no_measurement_not_a_zero() {
        assert!(timing_from_rows(&[]).is_none());
    }

    /// A run with no model rows at all (an FFI-free serve that never records usage)
    /// still yields a timing, with a zero model share — and that IS the honest reading:
    /// the window has motes, none of them was a model.
    #[test]
    fn a_window_with_no_model_rows_reports_zero_model_time() {
        let rows = [row(10, 0, 40, "", "mcp-echo/echo")];
        let t = timing_from_rows(&rows).expect("a non-empty window measures");
        assert_eq!(t.model_ms, 0);
        assert_eq!(t.total_ms, 40, "the span is still the span");
    }

    /// The spikes never gate, and they must say how much of the suite they cover: a p95
    /// computed over two of sixteen tasks is not a suite number. `measured_tasks` is what
    /// stops a partial measurement reading as full coverage.
    #[test]
    fn latency_spikes_report_their_own_coverage() {
        let with = |ms: Option<u64>| Transcript {
            task_id: "t".into(),
            turns: vec![],
            final_answer: None,
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 6,
            timing: ms.map(|total_ms| TranscriptTiming {
                total_ms,
                model_ms: total_ms / 2,
                output_tokens: Some(total_ms), // 1 token/ms — an easy sum to pin below
            }),
        };
        let spikes = latency_spikes(&[with(Some(100)), with(None), with(Some(300))]);
        let get = |id: &str| {
            spikes
                .iter()
                .find(|s| s.metric_id == id)
                .and_then(|s| match &s.value {
                    kx_eval::ScoreValue::Spike { value, .. } => Some(*value),
                    kx_eval::ScoreValue::Gate { .. } => None,
                })
        };
        assert_eq!(
            get("measured_tasks"),
            Some(2.0),
            "the unmeasured task is not counted"
        );
        assert_eq!(get("suite_latency_ms_total"), Some(400.0));
        assert_eq!(get("task_latency_ms_max"), Some(300.0));
        assert!(
            spikes
                .iter()
                .all(|s| matches!(s.value, kx_eval::ScoreValue::Spike { .. })),
            "every latency number is a Spike — none of them may ever gate"
        );
    }

    /// With nothing measured at all, the coverage spike still reports — as a zero — so a
    /// run that measured nothing is distinguishable from a run that was never asked to.
    #[test]
    fn latency_spikes_survive_a_suite_that_measured_nothing() {
        let spikes = latency_spikes(&[]);
        assert_eq!(spikes.len(), 1, "only the coverage count");
        assert_eq!(spikes[0].metric_id, "measured_tasks");
    }

    /// The token economy: attributed like the timing, covered by its own count, and
    /// `tokens_per_success` OMITTED — not zero, not infinity — when nothing passed.
    #[test]
    fn token_spikes_cover_count_mean_family_and_success() {
        let transcript = |tokens: Option<u64>| Transcript {
            task_id: "t".into(),
            turns: vec![],
            final_answer: None,
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 6,
            timing: tokens.map(|output| TranscriptTiming {
                total_ms: 100,
                model_ms: 50,
                output_tokens: Some(output),
            }),
        };
        let scored = |family: &str, success: u32| kx_eval::TaskScore {
            task_id: "t".into(),
            family: family.into(),
            scores: vec![ScoreOutput::gate("task_success", success, "")],
        };
        let per_task = [
            scored("tool", 1000),
            scored("tool", 0),
            scored("http", 1000),
        ];
        let transcripts = [
            transcript(Some(200)),
            transcript(Some(400)),
            transcript(None), // measured nothing — out of every mean and denominator
        ];
        let spikes = token_spikes(&per_task, &transcripts);
        let get = |id: &str| {
            spikes
                .iter()
                .find(|s| s.metric_id == id)
                .and_then(|s| match &s.value {
                    kx_eval::ScoreValue::Spike { value, .. } => Some(*value),
                    kx_eval::ScoreValue::Gate { .. } => None,
                })
        };
        assert_eq!(get("tokens_measured_tasks"), Some(2.0));
        assert_eq!(get("tokens_per_task_mean"), Some(300.0));
        assert_eq!(get("tokens_per_task_mean@tool"), Some(300.0));
        assert_eq!(
            get("tokens_per_task_mean@http"),
            None,
            "a family whose tasks reported no counts publishes no mean"
        );
        // One measured pass (the 200-token task) carries the whole measured total.
        assert_eq!(get("tokens_per_success"), Some(600.0));
        assert!(
            spikes
                .iter()
                .all(|s| matches!(s.value, kx_eval::ScoreValue::Spike { .. })),
            "every token number is a Spike — none of them may ever gate"
        );
    }

    /// The absence rule (a cost-per-success with zero successes is no number at all).
    #[test]
    fn tokens_per_success_is_omitted_when_nothing_passed() {
        let per_task = [kx_eval::TaskScore {
            task_id: "t".into(),
            family: "tool".into(),
            scores: vec![ScoreOutput::gate("task_success", 0, "")],
        }];
        let transcripts = [Transcript {
            task_id: "t".into(),
            turns: vec![],
            final_answer: None,
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 6,
            timing: Some(TranscriptTiming {
                total_ms: 100,
                model_ms: 50,
                output_tokens: Some(500),
            }),
        }];
        let spikes = token_spikes(&per_task, &transcripts);
        assert!(spikes.iter().any(|s| s.metric_id == "tokens_per_task_mean"));
        assert!(
            spikes.iter().all(|s| s.metric_id != "tokens_per_success"),
            "no successes ⇒ the spike is absent, never a division by zero or a zero"
        );
    }

    fn bench_task(id: &str) -> GoldenTask {
        kx_eval::load_bench_v1()
            .expect("bench-v1 corpus loads")
            .suite
            .tasks
            .into_iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("bench-v1 has a task {id:?}"))
    }

    /// The corpus and the runner must agree: every SHIPPED task resolves to a shape.
    /// A task nobody can drive is coverage on paper.
    #[test]
    fn every_bench_v1_task_resolves_to_a_drive_shape() {
        let corpus = kx_eval::load_bench_v1().expect("bench-v1 corpus loads");
        for task in &corpus.suite.tasks {
            drive_for(task).unwrap_or_else(|e| panic!("task {} has no drive shape: {e}", task.id));
        }
    }

    /// The three `reach` tasks are three genuinely different paths, not three prompts:
    /// a retrieval recipe, a memory recipe, and an App whose steering inherits the
    /// principal's ceiling. If two collapsed to the same shape the family would be
    /// measuring one thing three times.
    #[test]
    fn the_reach_family_drives_three_distinct_shapes() {
        assert_eq!(
            drive_for(&bench_task("rag-grounded-answer")).unwrap(),
            Drive::React {
                handle: REACT_RAG_RECIPE_HANDLE,
                dataset: Some(BENCH_DATASET),
            }
        );
        assert_eq!(
            drive_for(&bench_task("memory-recall")).unwrap(),
            Drive::React {
                handle: REACT_MEMORY_RECIPE_HANDLE,
                dataset: None,
            }
        );
        assert_eq!(
            drive_for(&bench_task("reach-inherit-principal")).unwrap(),
            Drive::App {
                handle: BENCH_REACH_APP_HANDLE,
            }
        );
        assert_eq!(
            drive_for(&bench_task("swarm-fanout-gather")).unwrap(),
            Drive::Swarm
        );
        // The tool-contract families share the autogranted loop.
        assert_eq!(
            drive_for(&bench_task("tool-contract-refusal")).unwrap(),
            Drive::React {
                handle: REACT_AUTO_RECIPE_HANDLE,
                dataset: None,
            }
        );
    }

    /// An unrecognized family must FAIL, not fall through to the react path — a task
    /// driven down the wrong shape still produces a plausible number.
    #[test]
    fn an_unknown_family_fails_closed() {
        let mut task = bench_task("kv-lookup-x");
        task.family = "telepathy".into();
        assert!(matches!(
            drive_for(&task),
            Err(BenchError::UnknownFamily { .. })
        ));
        // Likewise a reach task nobody wired a fixture for.
        let mut orphan = bench_task("rag-grounded-answer");
        orphan.id = "reach-something-new".into();
        assert!(matches!(
            drive_for(&orphan),
            Err(BenchError::UnknownFamily { .. })
        ));
    }

    /// The shipped swarm instruction lowers to N leaves fanned into ONE gather: every
    /// leaf is a Data parent of the last step, and the leaves have no edges between them.
    #[test]
    fn a_swarm_instruction_lowers_to_fanout_gather() {
        let task = bench_task("swarm-fanout-gather");
        let req = swarm_request(&task.instruction);
        assert_eq!(req.steps.len(), 4, "3 agents + 1 gather");
        let gather = 3;
        assert_eq!(req.edges.len(), 3, "one Data edge per agent");
        for e in &req.edges {
            assert_eq!(e.child, gather, "every edge fans INTO the gather");
            assert_eq!(e.edge_kind, proto::EdgeKind::Data as i32);
            assert!(e.parent < gather, "leaves precede the gather");
        }
        // Distinct agents — a lowering that duplicated one prompt would still fan in.
        let prompts: BTreeSet<&str> = req.steps.iter().map(|s| s.prompt.as_str()).collect();
        assert_eq!(prompts.len(), 4, "each participant carries its own prompt");
        // Every step is a MODEL step (a swarm is composition, not a new step kind).
        assert!(req
            .steps
            .iter()
            .all(|s| s.kind == proto::WorkflowStepKind::Model as i32));
    }

    /// The lowering is total but never INVENTS a fan-out: a single-segment instruction
    /// lowers to one step and no edges. Manufacturing a synthetic leaf would let the
    /// swarm gate pass on a chain that never fanned out.
    #[test]
    fn a_single_segment_swarm_is_not_given_a_synthetic_fanout() {
        let req = swarm_request("just do the thing");
        assert_eq!(req.steps.len(), 1);
        assert!(req.edges.is_empty());
    }

    /// …which is why the CORPUS carries the contract: every swarm task must declare at
    /// least two agents plus a gather, so the fold has a real fan-in to walk.
    #[test]
    fn a_swarm_task_declares_a_real_fanout() {
        let corpus = kx_eval::load_bench_v1().expect("bench-v1 corpus loads");
        for task in corpus.suite.tasks.iter().filter(|t| t.family == "swarm") {
            let req = swarm_request(&task.instruction);
            assert!(
                req.steps.len() >= 3,
                "swarm task {} must declare >= 2 agents + a gather (got {} steps)",
                task.id,
                req.steps.len()
            );
            let gather = u32::try_from(req.steps.len() - 1).unwrap();
            assert!(
                req.edges.len() >= 2 && req.edges.iter().all(|e| e.child == gather),
                "swarm task {} must fan every agent into one gather",
                task.id
            );
        }
    }

    /// Observations are fetched only where an oracle reads them — the grounded and
    /// recall tasks — and never for the tool families (two RPCs per tool turn).
    #[test]
    fn observations_are_fetched_only_where_an_oracle_reads_them() {
        assert_eq!(
            observation_tools_for(&bench_task("rag-grounded-answer")),
            ["retrieve".to_string()].into_iter().collect()
        );
        assert_eq!(
            observation_tools_for(&bench_task("memory-recall")),
            ["recall".to_string()].into_iter().collect()
        );
        assert!(observation_tools_for(&bench_task("kv-lookup-x")).is_empty());
        assert!(observation_tools_for(&bench_task("swarm-fanout-gather")).is_empty());
    }

    /// The attribution filter cannot become a way to ratchet the corpus against a subset.
    ///
    /// A filtered run reports every held-back task as SKIPPED, so `is_complete()` is
    /// false and the driver's capture guard refuses it — the same mechanism that already
    /// protects against an unprovisioned family. Without this the filter would be a
    /// quiet path to a baseline over three tasks that reads as full coverage forever.
    #[test]
    fn a_filtered_run_can_never_be_captured_as_a_baseline() {
        let report = kx_eval::aggregate(
            "bench-v1".into(),
            "digest".into(),
            vec![],
            &ScoreOutput::not_applicable("format_coverage", "N/A"),
            &[],
            "env".into(),
            "sha".into(),
        );
        let filtered = LiveSuiteOutcome {
            report,
            skipped: vec![SkippedFamily {
                family: "tool".into(),
                missing_recipe: FILTERED_OUT.into(),
                task_ids: vec!["kv-lookup-x".into()],
            }],
            transcripts: vec![],
        };
        assert!(
            !filtered.is_complete(),
            "a KX_BENCH_ONLY run is INCOMPLETE by construction"
        );
        assert!(
            filtered.skipped[0].missing_recipe.contains("KX_BENCH_ONLY"),
            "and it says WHY, so a filtered run is never mistaken for missing coverage"
        );
    }

    /// The baseline guard: an outcome that skipped a family is NOT complete, so the
    /// driver refuses to capture it as the committed ratchet.
    #[test]
    fn an_outcome_that_skipped_a_family_is_incomplete() {
        let report = kx_eval::aggregate(
            "bench-v1".into(),
            "digest".into(),
            vec![],
            &ScoreOutput::not_applicable("format_coverage", "N/A"),
            &[],
            "env".into(),
            "sha".into(),
        );
        let complete = LiveSuiteOutcome {
            report: report.clone(),
            skipped: vec![],
            transcripts: vec![],
        };
        assert!(complete.is_complete());
        let partial = LiveSuiteOutcome {
            report,
            skipped: vec![SkippedFamily {
                family: "reach".into(),
                missing_recipe: REACT_RAG_RECIPE_HANDLE.into(),
                task_ids: vec!["rag-grounded-answer".into()],
            }],
            transcripts: vec![],
        };
        assert!(!partial.is_complete());
    }

    #[test]
    fn wire_branches_map_to_eval_branches() {
        assert_eq!(branch_from_wire("answer"), Branch::Answer);
        assert_eq!(branch_from_wire("tool"), Branch::Tool);
        assert_eq!(branch_from_wire("rejected"), Branch::Rejected);
        assert_eq!(branch_from_wire("dead_lettered"), Branch::DeadLettered);
        // An in-flight or unknown wire string is not a terminal verdict.
        assert_eq!(branch_from_wire("pending"), Branch::Pending);
        assert_eq!(branch_from_wire(""), Branch::Pending);
    }
}
