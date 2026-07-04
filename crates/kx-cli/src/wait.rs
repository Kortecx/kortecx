//! `--wait` orchestration: turn an async run handle into a single result by
//! composing existing RPCs client-side — poll `GetProjection` until the target
//! Mote reaches a terminal state, then `GetContent` its committed result. This is
//! what lets an agent call the runtime like a function (one command in, one
//! parseable result out) without managing a stream. No new server capability is
//! used; it is forward-compatible with R5's live events (the poll loop becomes a
//! subscription, the [`WaitOutcome`] is unchanged).
//!
//! Two entry points: [`await_result`] targets a specific `terminal_mote_id` (the
//! `invoke` path, which gets one from `InvokeResponse`); [`await_any_result`]
//! waits for the first committed Mote in a run (the `submit` path, whose
//! `RunHandle` carries no terminal id).

use std::time::Duration;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

use crate::client::Resolved;
use crate::error::CliError;

/// Polling cadence while waiting (bounded backoff — never a busy-spin). Same
/// order of magnitude as the embedded worker's idle poll.
const POLL: Duration = Duration::from_millis(250);

/// The terminal disposition of a waited-on run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitState {
    /// The target Mote committed; [`WaitOutcome::payload`] holds its result.
    Committed,
    /// The target Mote reached a failure/anomaly state.
    Failed,
    /// The timeout elapsed before a terminal state — the run is still in
    /// progress and resumable (via `kx projection` / `kx events`).
    Running,
}

/// The result of a `--wait`: server-derived ids + the terminal disposition and,
/// when committed, the result ref + payload.
#[derive(Debug, Clone)]
pub struct WaitOutcome {
    /// The run the invocation joined (16B).
    pub instance_id: Vec<u8>,
    /// The server-derived terminal/sink Mote (32B); empty if a failure/timeout
    /// occurred before any Mote was identified (the `submit` path).
    pub terminal_mote_id: Vec<u8>,
    /// The terminal disposition.
    pub state: WaitState,
    /// The committed result ref (32B), present iff `state == Committed`.
    pub result_ref: Option<Vec<u8>>,
    /// The committed result bytes, fetched via `GetContent` iff `Committed`.
    pub payload: Option<Vec<u8>>,
}

/// `true` for a state we should keep polling (not yet terminal).
fn is_pending(state: i32) -> bool {
    state == proto::MoteSnapshotState::Pending as i32
        || state == proto::MoteSnapshotState::Scheduled as i32
}

fn is_committed(state: i32) -> bool {
    state == proto::MoteSnapshotState::Committed as i32
}

async fn get_projection(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    instance_id: &[u8],
) -> Result<proto::ProjectionView, CliError> {
    Ok(client
        .get_projection(resolved.request(proto::GetProjectionRequest {
            instance_id: instance_id.to_vec(),
            at_seq: None,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner())
}

async fn fetch_payload(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    instance_id: &[u8],
    content_ref: &[u8],
) -> Result<Vec<u8>, CliError> {
    Ok(client
        .get_content(resolved.request(proto::GetContentRequest {
            content_ref: content_ref.to_vec(),
            instance_id: instance_id.to_vec(),
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner()
        .payload)
}

/// Build a `Committed` outcome, fetching the result content.
async fn committed_outcome(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    instance_id: Vec<u8>,
    mote_id: Vec<u8>,
    result_ref: Option<Vec<u8>>,
) -> Result<WaitOutcome, CliError> {
    let payload = match &result_ref {
        Some(content_ref) => {
            Some(fetch_payload(client, resolved, &instance_id, content_ref).await?)
        }
        None => None,
    };
    Ok(WaitOutcome {
        instance_id,
        terminal_mote_id: mote_id,
        state: WaitState::Committed,
        result_ref,
        payload,
    })
}

fn terminal(instance_id: Vec<u8>, mote_id: Vec<u8>, state: WaitState) -> WaitOutcome {
    WaitOutcome {
        instance_id,
        terminal_mote_id: mote_id,
        state,
        result_ref: None,
        payload: None,
    }
}

/// Poll until `terminal_mote_id` is terminal (or `timeout` elapses), fetching
/// the committed content on success. Every RPC carries the resolved bearer token.
pub async fn await_result(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    instance_id: Vec<u8>,
    terminal_mote_id: Vec<u8>,
    timeout: Duration,
) -> Result<WaitOutcome, CliError> {
    let start = tokio::time::Instant::now();
    loop {
        let view = get_projection(client, resolved, &instance_id).await?;
        if let Some((state, result_ref)) = view
            .motes
            .iter()
            .find(|m| m.mote_id == terminal_mote_id)
            .map(|m| (m.state, m.result_ref.clone()))
        {
            if is_committed(state) {
                return committed_outcome(
                    client,
                    resolved,
                    instance_id,
                    terminal_mote_id,
                    result_ref,
                )
                .await;
            } else if !is_pending(state) {
                return Ok(terminal(instance_id, terminal_mote_id, WaitState::Failed));
            }
        }
        if start.elapsed() >= timeout {
            return Ok(terminal(instance_id, terminal_mote_id, WaitState::Running));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Poll until ANY Mote in the run commits (the `submit` path: a `RunHandle` has
/// no terminal id). If every Mote reaches a terminal non-committed state, the
/// run is `Failed`; on timeout it is `Running`.
pub async fn await_any_result(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    instance_id: Vec<u8>,
    timeout: Duration,
) -> Result<WaitOutcome, CliError> {
    let start = tokio::time::Instant::now();
    loop {
        let view = get_projection(client, resolved, &instance_id).await?;
        if let Some((mote_id, result_ref)) = view
            .motes
            .iter()
            .find(|m| is_committed(m.state))
            .map(|m| (m.mote_id.clone(), m.result_ref.clone()))
        {
            return committed_outcome(client, resolved, instance_id, mote_id, result_ref).await;
        }
        // All Motes terminal (and none committed) ⇒ the run failed.
        if !view.motes.is_empty() && view.motes.iter().all(|m| !is_pending(m.state)) {
            let first = view
                .motes
                .first()
                .map(|m| m.mote_id.clone())
                .unwrap_or_default();
            return Ok(terminal(instance_id, first, WaitState::Failed));
        }
        if start.elapsed() >= timeout {
            return Ok(terminal(instance_id, Vec::new(), WaitState::Running));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The branch a settled ReAct turn freezes to (vs the live `pending` / `tool` states).
const REACT_ANSWER: &str = "answer";
const REACT_DEAD: &str = "dead_lettered";

/// Wait for a ReAct CHAIN to settle (the `kx/recipes/react` path). A react chain
/// has no statically-known terminal Mote — the run-salted turn-0 id the gateway
/// returns never commits, and the settled Answer turn is unknown until the model
/// emits it — so completion is observed via `ListReactTurns`: done when a turn
/// settles to `answer` (resolve its committed content) or `dead_lettered` (terminal
/// failure). Mirrors the SDK `poll_react_result` (campaign finding F13). The
/// `agent run` verb collects the tool ACTIONS with a final `ListReactTurns`.
pub async fn await_react_result(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    instance_id: Vec<u8>,
    chain_salt: Vec<u8>,
    timeout: Duration,
) -> Result<WaitOutcome, CliError> {
    // PR-R1: scope the poll to THIS invocation's chain (serve shares one journal /
    // instance_id across every Invoke). A 32-byte `react_chain_salt` from Invoke
    // selects the chain; an EMPTY salt (a non-react run, or an old server) falls back
    // to instance_id-only scoping (the pre-PR-R1 behaviour).
    let step_salt = (!chain_salt.is_empty()).then(|| chain_salt.clone());
    let start = tokio::time::Instant::now();
    loop {
        let turns = client
            .list_react_turns(resolved.request(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(instance_id.clone()),
                step_salt: step_salt.clone(),
            })?)
            .await
            .map_err(CliError::from_status)?
            .into_inner()
            .turns;
        if let Some(answer) = turns.iter().find(|t| t.branch == REACT_ANSWER) {
            // The answer turn's committed result lives at its `result_ref` in the
            // projection (the run-salted turn id is not statically known up front).
            let view = get_projection(client, resolved, &instance_id).await?;
            let result_ref = view
                .motes
                .iter()
                .find(|m| m.mote_id == answer.turn_mote_id)
                .and_then(|m| m.result_ref.clone());
            let mut outcome = committed_outcome(
                client,
                resolved,
                instance_id,
                answer.turn_mote_id.clone(),
                result_ref,
            )
            .await?;
            // gemma3 connector-tool-fire: under the Ollama non-strict UNION `format` the
            // settled answer turn commits `{"answer":"…"}`; unwrap it to the plain text a
            // user expects. Byte-identical NO-OP for prose / llama.cpp answers (SN-8).
            if let Some(payload) = outcome.payload.take() {
                outcome.payload = Some(kx_toolcall::extract_answer(&payload).into_owned());
            }
            return Ok(outcome);
        }
        if let Some(dead) = turns.iter().find(|t| t.branch == REACT_DEAD) {
            return Ok(terminal(
                instance_id,
                dead.turn_mote_id.clone(),
                WaitState::Failed,
            ));
        }
        if start.elapsed() >= timeout {
            return Ok(terminal(instance_id, Vec::new(), WaitState::Running));
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_predicates() {
        assert!(is_pending(proto::MoteSnapshotState::Pending as i32));
        assert!(is_pending(proto::MoteSnapshotState::Scheduled as i32));
        assert!(!is_pending(proto::MoteSnapshotState::Committed as i32));
        assert!(is_committed(proto::MoteSnapshotState::Committed as i32));
        assert!(!is_committed(proto::MoteSnapshotState::Failed as i32));
    }
}
