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
//! Reusable by the real-model benchmark driver and its later coverage lane: both drive
//! [`score_live_suite`]; neither re-implements the fold.

use std::time::Duration;

use kx_eval::{
    aggregate, score_transcript, BenchCorpus, Branch, EvalReport, GoldenTask, ScoreInput,
    ScoreOutput, TaskScore, Transcript, TurnRecord,
};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

use crate::provision::REACT_AUTO_RECIPE_HANDLE;

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
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchError::Rpc(method, status) => write!(f, "{method} rpc failed: {status}"),
            BenchError::EncodeArgs(e) => write!(f, "encode invoke args: {e}"),
            BenchError::NotSettled(id) => {
                write!(f, "task {id:?} did not settle a terminal branch in time")
            }
        }
    }
}

impl std::error::Error for BenchError {}

/// Fold ONE settled live run into a full [`Transcript`] — the trajectory from
/// `ListReactTurns` (scoped to this chain) plus the committed final answer from the run's
/// answer-branch turn.
///
/// `chain_salt` is `InvokeResponse.react_chain_salt` (empty ⇒ the legacy run-level chain).
/// The answer text is the committed content of the LAST `answer`-branch turn's Mote — NOT
/// the invocation's recipe sink (`terminal_mote_id`), whose committed value is a fold
/// wrapper, not the model's prose. RAG `retrieved_docs` / `rerank` are left empty in v1
/// (their families are a later coverage lane).
///
/// # Errors
/// [`BenchError::Rpc`] if any read RPC fails.
pub async fn fold_run_transcript(
    client: &mut KxGatewayClient<Channel>,
    instance_id: Vec<u8>,
    chain_salt: Vec<u8>,
    task_id: String,
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

    Ok(Transcript {
        task_id,
        turns,
        final_answer,
        retrieved_docs: Vec::new(),
        rerank: None,
        max_turns,
        max_tool_calls,
    })
}

/// Drive a whole bench suite on a served model and score every task against its
/// `Expectation`, returning the aggregate [`EvalReport`]. Each task is invoked, polled to
/// a terminal branch (bounded by `settle_timeout`), folded, and scored; `format_coverage`
/// is N/A for a live suite (it measures the static parse corpus, not a run).
///
/// # Errors
/// The first task that fails to invoke, settle, or fold aborts the suite with its
/// [`BenchError`].
pub async fn score_live_suite(
    client: &mut KxGatewayClient<Channel>,
    corpus: &BenchCorpus,
    env_label: String,
    git_sha: String,
    settle_timeout: Duration,
) -> Result<EvalReport, BenchError> {
    let mut per_task: Vec<TaskScore> = Vec::with_capacity(corpus.suite.tasks.len());
    for task in &corpus.suite.tasks {
        let transcript = run_and_fold(client, task, settle_timeout).await?;
        let scores = score_transcript(&ScoreInput {
            transcript: &transcript,
            expect: &task.expect,
        });
        per_task.push(TaskScore {
            task_id: task.id.clone(),
            scores,
        });
    }
    let format_na = ScoreOutput::not_applicable("format_coverage", "N/A for a live suite");
    Ok(aggregate(
        corpus.suite.id.clone(),
        corpus.suite_digest.clone(),
        per_task,
        &format_na,
        &[],
        env_label,
        git_sha,
    ))
}

/// Invoke `react-auto` with a task's instruction, wait for the chain to settle, and fold
/// the run into a transcript.
async fn run_and_fold(
    client: &mut KxGatewayClient<Channel>,
    task: &GoldenTask,
    settle_timeout: Duration,
) -> Result<Transcript, BenchError> {
    let args = serde_json::to_vec(&serde_json::json!({
        "instruction": task.instruction,
        "max_turns": BENCH_MAX_TURNS,
        "max_tool_calls": BENCH_MAX_TOOL_CALLS,
    }))
    .map_err(BenchError::EncodeArgs)?;

    let resp = client
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args,
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .map_err(|e| BenchError::Rpc("invoke", e))?
        .into_inner();

    let step_salt = (!resp.react_chain_salt.is_empty()).then(|| resp.react_chain_salt.clone());
    let polls = (settle_timeout.as_millis() / 100).max(1);
    let mut settled = false;
    for _ in 0..polls {
        let t = client
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
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
        resp.instance_id,
        resp.react_chain_salt,
        task.id.clone(),
    )
    .await
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
