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
    ScoreOutput, TaskScore, Transcript, TurnRecord,
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
        "tool" | "react" => Ok(Drive::React {
            handle: REACT_AUTO_RECIPE_HANDLE,
            dataset: None,
        }),
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
    for task in &corpus.suite.tasks {
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
        let transcript = run_and_fold(client, task, &drive, settle_timeout).await?;
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
    Ok(LiveSuiteOutcome {
        report: aggregate(
            corpus.suite.id.clone(),
            corpus.suite_digest.clone(),
            per_task,
            &format_na,
            &[],
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

/// Drive ONE task through the shape its family names, and fold the settled run.
async fn run_and_fold(
    client: &mut KxGatewayClient<Channel>,
    task: &GoldenTask,
    drive: &Drive,
    settle_timeout: Duration,
) -> Result<Transcript, BenchError> {
    match drive {
        Drive::React { handle, dataset } => {
            let mut args = serde_json::json!({
                "instruction": task.instruction,
                "max_turns": BENCH_MAX_TURNS,
                "max_tool_calls": BENCH_MAX_TOOL_CALLS,
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
            fold_workflow_transcript(
                client,
                handle.instance_id,
                handle.terminal_mote_id,
                task.id.clone(),
                settle_timeout,
            )
            .await
        }
    }
}

/// Poll a ReAct chain to a terminal branch, then fold it.
async fn settle_and_fold_react(
    client: &mut KxGatewayClient<Channel>,
    instance_id: Vec<u8>,
    chain_salt: Vec<u8>,
    task: &GoldenTask,
    settle_timeout: Duration,
) -> Result<Transcript, BenchError> {
    let step_salt = (!chain_salt.is_empty()).then(|| chain_salt.clone());
    let polls = (settle_timeout.as_millis() / 100).max(1);
    let mut settled = false;
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
        if t.turns
            .iter()
            .any(|x| x.branch == "answer" || x.branch == "dead_lettered")
        {
            settled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !settled {
        return Err(BenchError::NotSettled(task.id.clone()));
    }

    fold_run_transcript(
        client,
        instance_id,
        chain_salt,
        task.id.clone(),
        &observation_tools_for(task),
    )
    .await
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
