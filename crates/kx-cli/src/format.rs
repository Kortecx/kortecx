//! Rendering for the client verbs — a human-readable default and an opt-in
//! `--json` machine form. All ids / refs / digests are lowercase hex. The
//! functions return `String`s (so they're unit-testable); the verbs do the
//! actual stdout write. `content`'s raw-bytes path is the one exception (the
//! verb writes the payload directly — see [`crate::verbs::content`]).

use std::fmt::Write as _;

use kx_proto::proto;
use serde_json::{json, Value};

use crate::hex;
use crate::wait::{WaitOutcome, WaitState};

/// Map a [`proto::MoteSnapshotState`] discriminant to a stable display name.
/// An out-of-range value renders `UNKNOWN` (forward-compatible with a future
/// proto that adds a state — no panic, no silent mis-label).
#[must_use]
pub fn state_name(state: i32) -> &'static str {
    use proto::MoteSnapshotState as S;
    if state == S::Pending as i32 {
        "PENDING"
    } else if state == S::Scheduled as i32 {
        "SCHEDULED"
    } else if state == S::Committed as i32 {
        "COMMITTED"
    } else if state == S::Failed as i32 {
        "FAILED"
    } else if state == S::Repudiated as i32 {
        "REPUDIATED"
    } else if state == S::Inconsistent as i32 {
        "INCONSISTENT"
    } else {
        "UNKNOWN"
    }
}

/// Map a [`proto::EdgeKind`] discriminant to a short edge label for the human
/// projection rendering (`data`/`control`). Out-of-range renders `unknown`
/// (forward-compatible). The `--json` form keeps the raw discriminant for parity
/// with the Python SDK (`MoteView`).
#[must_use]
pub fn edge_kind_name(edge_kind: i32) -> &'static str {
    use proto::EdgeKind as E;
    if edge_kind == E::Data as i32 {
        "data"
    } else if edge_kind == E::Control as i32 {
        "control"
    } else {
        "unknown"
    }
}

/// Render an `invoke` (no `--wait`) result: the async run handle.
#[must_use]
pub fn render_invoke(resp: &proto::InvokeResponse, json: bool) -> String {
    if json {
        json!({
            "instance_id": hex::encode(&resp.instance_id),
            "recipe_fingerprint": hex::encode(&resp.recipe_fingerprint),
            "terminal_mote_id": hex::encode(&resp.terminal_mote_id),
        })
        .to_string()
    } else {
        format!(
            "instance_id        {}\nrecipe_fingerprint {}\nterminal_mote_id   {}",
            hex::encode(&resp.instance_id),
            hex::encode(&resp.recipe_fingerprint),
            hex::encode(&resp.terminal_mote_id),
        )
    }
}

/// Render a `submit` (no `--wait`) run handle.
#[must_use]
pub fn render_submit(handle: &proto::RunHandle, json: bool) -> String {
    if json {
        json!({
            "instance_id": hex::encode(&handle.instance_id),
            "recipe_fingerprint": hex::encode(&handle.recipe_fingerprint),
        })
        .to_string()
    } else {
        format!(
            "instance_id        {}\nrecipe_fingerprint {}",
            hex::encode(&handle.instance_id),
            hex::encode(&handle.recipe_fingerprint),
        )
    }
}

/// Render a projection view (the run rendered as a DAG of Mote states).
#[must_use]
pub fn render_projection(view: &proto::ProjectionView, json: bool) -> String {
    if json {
        let motes: Vec<Value> = view
            .motes
            .iter()
            .map(|m| {
                json!({
                    "mote_id": hex::encode(&m.mote_id),
                    "state": state_name(m.state),
                    "nd_class": m.nd_class,
                    "promotion": m.promotion,
                    "result_ref": m.result_ref.as_deref().map(hex::encode),
                    "committed_seq": m.committed_seq,
                    "anomaly": m.anomaly,
                    // The Mote's incoming DAG edges (server-derived projection
                    // topology). edge_kind is the stable NAME (data/control/unknown)
                    // — self-describing + byte-identical across the CLI/Python/TS
                    // --json shapes (the TS ParentEdge established the name form).
                    "parents": m.parents.iter().map(|p| json!({
                        "parent_id": hex::encode(&p.parent_id),
                        "edge_kind": edge_kind_name(p.edge_kind),
                        "non_cascade": p.non_cascade,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({
            "instance_id": hex::encode(&view.instance_id),
            "recipe_fingerprint": hex::encode(&view.recipe_fingerprint),
            "current_seq": view.current_seq,
            "motes": motes,
        })
        .to_string()
    } else {
        let mut out = format!(
            "instance {}  recipe {}  seq {}",
            hex::encode(&view.instance_id),
            hex::encode(&view.recipe_fingerprint),
            view.current_seq,
        );
        for m in &view.motes {
            let parents = if m.parents.is_empty() {
                "-".to_string()
            } else {
                m.parents
                    .iter()
                    .map(|p| {
                        format!(
                            "{}:{}",
                            &hex::encode(&p.parent_id)[..8.min(p.parent_id.len() * 2)],
                            edge_kind_name(p.edge_kind),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let _ = write!(
                out,
                "\n  {}  {:<12} nd={} result={} committed_seq={} parents={}",
                hex::encode(&m.mote_id),
                state_name(m.state),
                m.nd_class,
                hex::encode_opt(m.result_ref.as_deref()),
                m.committed_seq
                    .map_or_else(|| "-".to_string(), |s| s.to_string()),
                parents,
            );
        }
        out
    }
}

/// Render a single event delta as one human line / one NDJSON object. `None`
/// for a delta with no recognized `kind` (forward-compat: skip silently).
#[must_use]
pub fn render_delta(delta: &proto::EventDelta, json: bool) -> Option<String> {
    use proto::event_delta::Kind;
    let kind = delta.kind.as_ref()?;
    let seq = delta.seq;
    let line = match kind {
        Kind::Committed(c) => {
            if json {
                json!({"seq": seq, "kind": "committed", "mote_id": hex::encode(&c.mote_id),
                       "result_ref": hex::encode(&c.result_ref), "nd_class": c.nd_class})
                .to_string()
            } else {
                format!(
                    "seq {seq} COMMITTED  mote={} result={} nd={}",
                    hex::encode(&c.mote_id),
                    hex::encode(&c.result_ref),
                    c.nd_class
                )
            }
        }
        Kind::Failed(fd) => {
            if json {
                json!({"seq": seq, "kind": "failed", "mote_id": hex::encode(&fd.mote_id),
                       "reason_class": fd.reason_class})
                .to_string()
            } else {
                format!(
                    "seq {seq} FAILED     mote={} reason={}",
                    hex::encode(&fd.mote_id),
                    fd.reason_class
                )
            }
        }
        Kind::Repudiated(r) => {
            if json {
                json!({"seq": seq, "kind": "repudiated",
                       "target_mote_id": hex::encode(&r.target_mote_id),
                       "target_committed_seq": r.target_committed_seq})
                .to_string()
            } else {
                format!(
                    "seq {seq} REPUDIATED mote={} target_seq={}",
                    hex::encode(&r.target_mote_id),
                    r.target_committed_seq
                )
            }
        }
        Kind::EffectStaged(e) => {
            if json {
                json!({"seq": seq, "kind": "effect_staged", "mote_id": hex::encode(&e.mote_id)})
                    .to_string()
            } else {
                format!("seq {seq} EFFECT_STAGED mote={}", hex::encode(&e.mote_id))
            }
        }
    };
    Some(line)
}

/// Human display of a watermark-attribution instance hex: `-` when the delta
/// predates any registration (the wire keeps the honest empty string).
fn inst_display(inst_hex: &str) -> &str {
    if inst_hex.is_empty() {
        "-"
    } else {
        inst_hex
    }
}

/// Render one GLOBAL event delta (Batch C `StreamAllEvents`) as one human line /
/// one NDJSON object. `--json` field names mirror the WS `/events/all` wire
/// (the tri-surface parity contract): a `type` tag, a per-delta `instance_id`
/// (lowercase hex, EMPTY before any registration), lowercase `nd_class`, and an
/// honest `unknown` for a future delta kind (the per-run renderer skips those;
/// the global wire surfaces them).
#[must_use]
pub fn render_global_delta(delta: &proto::GlobalEventDelta, json: bool) -> String {
    use proto::global_event_delta::Kind;
    let seq = delta.seq;
    let inst = hex::encode(&delta.instance_id);
    match delta.kind.as_ref() {
        Some(Kind::Committed(c)) => {
            if json {
                json!({"seq": seq, "instance_id": inst, "type": "committed",
                       "mote_id": hex::encode(&c.mote_id),
                       "result_ref": hex::encode(&c.result_ref),
                       "nd_class": nd_class_tag(c.nd_class)})
                .to_string()
            } else {
                format!(
                    "seq {seq} COMMITTED  inst={} mote={} result={} nd={}",
                    inst_display(&inst),
                    hex::encode(&c.mote_id),
                    hex::encode(&c.result_ref),
                    nd_class_tag(c.nd_class)
                )
            }
        }
        Some(Kind::Failed(fd)) => {
            if json {
                json!({"seq": seq, "instance_id": inst, "type": "failed",
                       "mote_id": hex::encode(&fd.mote_id),
                       "reason_class": fd.reason_class})
                .to_string()
            } else {
                format!(
                    "seq {seq} FAILED     inst={} mote={} reason={}",
                    inst_display(&inst),
                    hex::encode(&fd.mote_id),
                    fd.reason_class
                )
            }
        }
        Some(Kind::Repudiated(r)) => {
            if json {
                json!({"seq": seq, "instance_id": inst, "type": "repudiated",
                       "target_mote_id": hex::encode(&r.target_mote_id),
                       "target_committed_seq": r.target_committed_seq})
                .to_string()
            } else {
                format!(
                    "seq {seq} REPUDIATED inst={} mote={} target_seq={}",
                    inst_display(&inst),
                    hex::encode(&r.target_mote_id),
                    r.target_committed_seq
                )
            }
        }
        Some(Kind::EffectStaged(e)) => {
            if json {
                json!({"seq": seq, "instance_id": inst, "type": "effect_staged",
                       "mote_id": hex::encode(&e.mote_id)})
                .to_string()
            } else {
                format!(
                    "seq {seq} EFFECT_STAGED inst={} mote={}",
                    inst_display(&inst),
                    hex::encode(&e.mote_id)
                )
            }
        }
        Some(Kind::RunRegistered(rr)) => {
            if json {
                json!({"seq": seq, "instance_id": inst, "type": "run_registered",
                       "recipe_fingerprint": hex::encode(&rr.recipe_fingerprint),
                       "registered_unix_ms": rr.registered_unix_ms})
                .to_string()
            } else {
                format!(
                    "seq {seq} RUN_STARTED inst={} recipe={} registered_ms={}",
                    inst_display(&inst),
                    hex::encode(&rr.recipe_fingerprint),
                    rr.registered_unix_ms
                )
            }
        }
        // A future delta kind: surface it honestly (the WS wire does the same).
        None => {
            if json {
                json!({"seq": seq, "instance_id": inst, "type": "unknown"}).to_string()
            } else {
                format!("seq {seq} UNKNOWN    inst={}", inst_display(&inst))
            }
        }
    }
}

/// T-AGENT2: decode a committed payload as a [`kx_critic_types::CriticVerdict`]
/// (the `kx/recipes/judge` terminal, or any critic mote's result) to a readable
/// `"valid"` / `"invalid: <reason>"` summary. Returns `None` for any payload that
/// is not a well-formed verdict (a model answer, a tool observation, …), so the
/// caller falls back to the raw UTF-8/hex display. Display-only (SN-8): the
/// summary never authorizes anything.
#[must_use]
pub fn critic_verdict_summary(payload: &[u8]) -> Option<String> {
    use kx_critic_types::{CriticReason, CriticVerdict};
    match CriticVerdict::decode(payload).ok()? {
        CriticVerdict::Valid => Some("valid".to_string()),
        CriticVerdict::Invalid { reason } => {
            let detail = match reason {
                CriticReason::JudgeRejected { reason_code: 0 } => {
                    "judge: answer did not satisfy the rubric".to_string()
                }
                CriticReason::JudgeRejected { reason_code: 1 } => {
                    "judge: response was unparseable (fail-closed)".to_string()
                }
                CriticReason::JudgeRejected { reason_code } => {
                    format!("judge: rejected (code {reason_code})")
                }
                CriticReason::SchemaMismatch { .. } => "schema mismatch".to_string(),
                CriticReason::DuplicateDetected { .. } => "duplicate detected".to_string(),
                CriticReason::StatOutOfBounds { .. } => "stat out of bounds".to_string(),
                CriticReason::PiiLeak { .. } => "PII leak".to_string(),
                CriticReason::Unparseable { .. } => "unparseable input".to_string(),
            };
            Some(format!("invalid: {detail}"))
        }
    }
}

/// Render the result of a `--wait` run. `include_payload` is `false` when the
/// caller wrote the payload to `--out` (then only metadata is emitted).
#[must_use]
pub fn render_wait(outcome: &WaitOutcome, json: bool, include_payload: bool) -> String {
    let state = match outcome.state {
        WaitState::Committed => "COMMITTED",
        WaitState::Failed => "FAILED",
        WaitState::Running => "RUNNING",
    };
    if json {
        // Build the map directly (no `as_object_mut().expect(...)` — lib code
        // denies `expect_used`).
        let mut map = serde_json::Map::new();
        map.insert(
            "instance_id".into(),
            json!(hex::encode(&outcome.instance_id)),
        );
        map.insert(
            "terminal_mote_id".into(),
            json!(hex::encode(&outcome.terminal_mote_id)),
        );
        map.insert("state".into(), json!(state));
        if let Some(ref_bytes) = &outcome.result_ref {
            map.insert("result_ref".into(), json!(hex::encode(ref_bytes)));
        }
        if let WaitState::Running = outcome.state {
            map.insert("timed_out".into(), json!(true));
        }
        if let Some(payload) = &outcome.payload {
            map.insert("result_len".into(), json!(payload.len()));
            // T-AGENT2: a judge/critic terminal commits a `CriticVerdict` — surface
            // the decoded VALID/INVALID summary alongside the raw bytes.
            if let Some(verdict) = critic_verdict_summary(payload) {
                map.insert("verdict".into(), json!(verdict));
            }
            if include_payload {
                if let Ok(text) = std::str::from_utf8(payload) {
                    map.insert("result_utf8".into(), json!(text));
                }
                map.insert("result_hex".into(), json!(hex::encode(payload)));
            }
        }
        Value::Object(map).to_string()
    } else {
        let mut out = format!(
            "instance_id      {}\nterminal_mote_id {}\nstate            {state}",
            hex::encode(&outcome.instance_id),
            hex::encode(&outcome.terminal_mote_id),
        );
        if let Some(ref_bytes) = &outcome.result_ref {
            let _ = write!(out, "\nresult_ref       {}", hex::encode(ref_bytes));
        }
        if let Some(payload) = &outcome.payload {
            let _ = write!(out, "\nresult_len       {}", payload.len());
            // T-AGENT2: a judge/critic terminal commits a `CriticVerdict` — show the
            // decoded VALID/INVALID summary (the raw bytes follow as result/hex).
            let verdict = critic_verdict_summary(payload);
            if let Some(ref v) = verdict {
                let _ = write!(out, "\nverdict          {v}");
            }
            if include_payload {
                match std::str::from_utf8(payload) {
                    // A judge verdict is non-UTF-8 bincode; the `verdict` line above
                    // is the readable form, so suppress the redundant raw hex for it.
                    Ok(text) => {
                        let _ = write!(out, "\nresult           {text}");
                    }
                    Err(_) if verdict.is_some() => {}
                    Err(_) => {
                        let _ = write!(out, "\nresult_hex       {}", hex::encode(payload));
                    }
                }
            }
        }
        out
    }
}

/// Render `kx agent run` (PR-9c-1) — the agent's final answer plus its AUDITED
/// tool-action set (the chain's settled `tool` turns, in order). `--json` mirrors
/// the SDK `AgentResult.json()` shape (`instance_id` / `run_handle` / `actions` /
/// `answer`); a non-committed disposition is surfaced honestly via `state`.
/// `actions` is a pre-filtered `(tool_id, tool_version, turn, call_index)` list — a
/// multi-call turn contributes several actions sharing `turn`, distinguished by
/// `call_index` (T-MULTI-ELEMENT-TOOLCALLS).
#[must_use]
pub fn render_agent_result(
    outcome: &WaitOutcome,
    actions: &[(String, String, u32, u32)],
    json: bool,
) -> String {
    if json {
        let action_vals: Vec<Value> = actions
            .iter()
            .map(|(id, ver, turn, call_index)| {
                json!({ "tool_id": id, "tool_version": ver, "turn": turn, "call_index": call_index })
            })
            .collect();
        let mut map = serde_json::Map::new();
        map.insert(
            "instance_id".into(),
            json!(hex::encode(&outcome.instance_id)),
        );
        // `run_handle` is the durable, re-attachable handle = the instance id.
        map.insert(
            "run_handle".into(),
            json!(hex::encode(&outcome.instance_id)),
        );
        map.insert("actions".into(), json!(action_vals));
        if let Some(payload) = &outcome.payload {
            if let Ok(text) = std::str::from_utf8(payload) {
                map.insert("answer".into(), json!(text));
            }
        }
        if outcome.state != WaitState::Committed {
            let st = match outcome.state {
                WaitState::Failed => "FAILED",
                WaitState::Running => "RUNNING",
                WaitState::Committed => "COMMITTED",
            };
            map.insert("state".into(), json!(st));
        }
        Value::Object(map).to_string()
    } else {
        let mut out = String::new();
        match &outcome.payload {
            Some(payload) => match std::str::from_utf8(payload) {
                Ok(text) => out.push_str(text),
                Err(_) => {
                    let _ = write!(out, "(binary answer, {} bytes)", payload.len());
                }
            },
            None => out.push_str(match outcome.state {
                // W2: a dead-lettered agent that fired tools but never settled looped
                // on its tool budget without answering — say so specifically (the
                // honest terminal, exit 1; never a masquerading "timed out; resume").
                WaitState::Failed if !actions.is_empty() => {
                    "(no answer — the agent exhausted its tool-call budget without settling on an answer)"
                }
                WaitState::Failed => "(no answer — the agent run failed)",
                WaitState::Running => {
                    "(no answer yet — timed out; resume with `kx react list --instance <id>`)"
                }
                WaitState::Committed => "(no answer payload)",
            }),
        }
        let _ = write!(out, "\n\nActions taken: {}", actions.len());
        for (id, ver, turn, call_index) in actions {
            // `turn N.call_index` so two parallel tools at turn 2 read `turn 2.0` /
            // `turn 2.1` (T-MULTI-ELEMENT-TOOLCALLS).
            let _ = write!(out, "\n  turn {turn}.{call_index}: {id}@{ver}");
        }
        let _ = write!(out, "\ninstance_id {}", hex::encode(&outcome.instance_id));
        out
    }
}

/// Render the JSON form of a fetched content blob (the human path writes raw
/// bytes; this is only used under `--json`).
#[must_use]
pub fn render_content_json(content_ref: &[u8], payload: &[u8]) -> String {
    json!({
        "content_ref": hex::encode(content_ref),
        "len": payload.len(),
        "payload_hex": hex::encode(payload),
    })
    .to_string()
}

/// Render `content put` — the server-derived ref + dedup flag (Batch A). SN-8:
/// the ref printed here came from the SERVER (blake3 over the payload), never a
/// client computation.
#[must_use]
pub fn render_put_content(resp: &proto::PutContentResponse, json: bool) -> String {
    if json {
        json!({
            "content_ref": hex::encode(&resp.content_ref),
            "size": resp.size,
            "deduplicated": resp.deduplicated,
        })
        .to_string()
    } else {
        format!(
            "ref={} size={} deduplicated={}",
            hex::encode(&resp.content_ref),
            resp.size,
            resp.deduplicated
        )
    }
}

/// Render `models list` — DISPLAY-ONLY discovery (SN-8: listing a model never
/// routes one; selection stays a recipe ENUM free-param).
#[must_use]
pub fn render_models(resp: &proto::ListModelsResponse, json: bool) -> String {
    if json {
        let models: Vec<Value> = resp
            .models
            .iter()
            .map(|m| {
                json!({
                    "model_id": m.model_id,
                    "modalities": m.modalities,
                    "description": m.description,
                    "serving": m.serving,
                    "context_len": m.context_len,
                    "loaded": m.loaded,
                    "chat_handle": m.chat_handle,
                    "engine": m.engine,
                    "can_embed": m.can_embed,
                    // Model Control v2 (additive).
                    "source": m.source,
                    "active": m.active,
                    "chat_rag_handle": m.chat_rag_handle,
                    // RC4a: true iff this (the configured embedder) is a decoder LLM.
                    "embed_is_decoder": m.embed_is_decoder,
                })
            })
            .collect();
        json!({ "models": models }).to_string()
    } else if resp.models.is_empty() {
        "(no models on this serve)".to_string()
    } else {
        // Model Control v2: group by engine so an Ollama ∥ llama.cpp split is obvious.
        let mut engines: Vec<&str> = resp.models.iter().map(|m| m.engine.as_str()).collect();
        engines.sort_unstable();
        engines.dedup();
        let mut out: Vec<String> = Vec::new();
        for eng in engines {
            let label = if eng.is_empty() {
                "models".to_string()
            } else {
                eng.strip_prefix("kx-").unwrap_or(eng).to_string()
            };
            out.push(format!("[{label}]"));
            for m in resp.models.iter().filter(|m| m.engine == eng) {
                out.push(format!(
                    "  {}  [{}]  ctx={}  {}{}{}{}{}{}",
                    m.model_id,
                    m.modalities.join("+"),
                    m.context_len,
                    m.description,
                    if m.serving { "  (serving)" } else { "" },
                    if m.active { "  (active)" } else { "" },
                    if m.loaded { "  (loaded)" } else { "" },
                    // RC4a: mark the embedder, flagging a decoder-as-embedder.
                    match (m.can_embed, m.embed_is_decoder) {
                        (true, true) => "  (embed·decoder — recommend a dedicated embed model)",
                        (true, false) => "  (embed)",
                        _ => "",
                    },
                    // Model Control v2: provenance of a pulled/runtime model.
                    match m.source.as_str() {
                        "pulled-ollama" | "pulled-url" => "  (pulled)",
                        _ => "",
                    },
                ));
            }
        }
        out.join("\n")
    }
}

/// Render `models pull` — the terminal pull status (Model Control v2).
#[must_use]
pub fn render_pull_status(
    model_id: &str,
    resp: &proto::GetPullStatusResponse,
    json: bool,
) -> String {
    let phase = pull_phase_label(resp.phase);
    if json {
        json!({
            "model_id": model_id,
            "phase": phase,
            "bytes_downloaded": resp.bytes_downloaded,
            "bytes_total": resp.bytes_total,
            "detail": resp.detail,
        })
        .to_string()
    } else {
        match proto::get_pull_status_response::Phase::try_from(resp.phase) {
            Ok(proto::get_pull_status_response::Phase::Done) => {
                format!("{model_id} pulled + registered (switch with `kx models use {model_id}`)")
            }
            Ok(proto::get_pull_status_response::Phase::Failed) => {
                format!("{model_id} pull FAILED: {}", resp.detail)
            }
            _ => format!("{model_id}: {phase} {}", resp.detail),
        }
    }
}

/// Render `models use` — the active-default switch (Model Control v2).
#[must_use]
pub fn render_set_active(resp: &proto::SetActiveModelResponse, json: bool) -> String {
    if json {
        json!({ "active_model_id": resp.active_model_id }).to_string()
    } else if resp.active_model_id.is_empty() {
        "active model cleared (back to the primary)".to_string()
    } else {
        format!("active model set to {}", resp.active_model_id)
    }
}

/// The display label for a `GetPullStatusResponse.phase` discriminant.
#[must_use]
fn pull_phase_label(phase: i32) -> &'static str {
    match proto::get_pull_status_response::Phase::try_from(phase) {
        Ok(proto::get_pull_status_response::Phase::Resolving) => "resolving",
        Ok(proto::get_pull_status_response::Phase::Downloading) => "downloading",
        Ok(proto::get_pull_status_response::Phase::Verifying) => "verifying",
        Ok(proto::get_pull_status_response::Phase::Registering) => "registering",
        Ok(proto::get_pull_status_response::Phase::Done) => "done",
        Ok(proto::get_pull_status_response::Phase::Failed) => "failed",
        _ => "unknown",
    }
}

/// Render `models load`/`offload` — the residency transition (POC-3).
#[must_use]
pub fn render_load_model(resp: &proto::LoadModelResponse, json: bool) -> String {
    if json {
        json!({
            "model_id": resp.model_id,
            "loaded": resp.loaded,
            "was_resident": resp.was_resident,
        })
        .to_string()
    } else if resp.was_resident {
        format!("{} already loaded", resp.model_id)
    } else {
        format!("{} loaded", resp.model_id)
    }
}

/// Render `models offload` — the residency transition (POC-3).
#[must_use]
pub fn render_offload_model(resp: &proto::OffloadModelResponse, json: bool) -> String {
    if json {
        json!({
            "model_id": resp.model_id,
            "loaded": resp.loaded,
            "was_resident": resp.was_resident,
        })
        .to_string()
    } else if resp.was_resident {
        format!("{} offloaded", resp.model_id)
    } else {
        format!("{} was not loaded", resp.model_id)
    }
}

/// Render `datasets list` — every RAG corpus on this serve (name, doc count, dim).
#[must_use]
pub fn render_datasets(resp: &proto::ListDatasetsResponse, json: bool) -> String {
    if json {
        let datasets: Vec<Value> = resp
            .datasets
            .iter()
            .map(|d| {
                json!({
                    "dataset_id": d.dataset_id,
                    "name": d.name,
                    "doc_count": d.doc_count,
                    "dim": d.dim,
                    "created_ms": d.created_ms,
                    "chunked": d.chunked,
                    "chunk_count": d.chunk_count,
                    "index_version": d.index_version,
                    "embed_model_fingerprint": d.embed_model_fingerprint,
                })
            })
            .collect();
        json!({ "datasets": datasets }).to_string()
    } else if resp.datasets.is_empty() {
        "(no datasets on this serve)".to_string()
    } else {
        resp.datasets
            .iter()
            .map(|d| {
                // RC4a: docs = parent documents; chunks = retrievable passages.
                let chunked = if d.chunked { "  chunked" } else { "" };
                format!(
                    "{}  docs={}  chunks={}  dim={}{}",
                    d.dataset_id, d.doc_count, d.chunk_count, d.dim, chunked
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `datasets ingest` — the post-dedup insert outcome.
#[must_use]
pub fn render_ingest(resp: &proto::IngestDocumentsResponse, json: bool) -> String {
    if json {
        json!({
            "dataset_id": resp.dataset_id,
            "doc_count": resp.doc_count,
            "inserted": resp.inserted,
            "dim": resp.dim,
        })
        .to_string()
    } else {
        format!(
            "dataset={} inserted={} doc_count={} dim={}",
            resp.dataset_id, resp.inserted, resp.doc_count, resp.dim
        )
    }
}

/// A short, single-line, control-char-free preview of a document's bytes (lossy
/// UTF-8, truncated). For the human form only — `--json` carries `text` in full.
fn doc_snippet(content: &[u8]) -> String {
    const MAX: usize = 80;
    let text: String = String::from_utf8_lossy(content)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = text.trim();
    if trimmed.chars().count() > MAX {
        let head: String = trimmed.chars().take(MAX).collect();
        format!("{head}…")
    } else {
        trimmed.to_string()
    }
}

/// Render `datasets query` hits. The `score` is DISPLAY-ONLY (SN-8) — a ranking
/// aid, never an identity input; the durable result is the ordered content-ref SET.
#[must_use]
pub fn render_dataset_hits(resp: &proto::QueryDatasetResponse, json: bool) -> String {
    if json {
        let hits: Vec<Value> = resp
            .hits
            .iter()
            .map(|h| {
                json!({
                    "content_ref": hex::encode(&h.content_ref),
                    "score": h.score,
                    "text": String::from_utf8_lossy(&h.content),
                    "parent_ref": hex::encode(&h.parent_ref),
                    "chunk_index": h.chunk_index,
                    "chunk_count": h.chunk_count,
                })
            })
            .collect();
        json!({ "hits": hits }).to_string()
    } else if resp.hits.is_empty() {
        "(no matches)".to_string()
    } else {
        resp.hits
            .iter()
            .map(|h| {
                // Show the leading 16 hex chars of the chunk ref + score + a snippet.
                // For a chunked corpus also show the passage position within its parent.
                let r = hex::encode(&h.content_ref);
                let short = r.get(..16).unwrap_or(&r);
                let chunk = if h.chunk_count > 1 {
                    format!("  [chunk {}/{}]", h.chunk_index + 1, h.chunk_count)
                } else {
                    String::new()
                };
                format!(
                    "{:.3}  {}{}  {}",
                    h.score,
                    short,
                    chunk,
                    doc_snippet(&h.content)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `signatures list`.
#[must_use]
pub fn render_signatures_list(resp: &proto::ListSignaturesResponse, json: bool) -> String {
    if json {
        let sigs: Vec<Value> = resp
            .signatures
            .iter()
            .map(|s| json!({"signature_id": hex::encode(&s.signature_id), "name": s.name}))
            .collect();
        json!({ "signatures": sigs }).to_string()
    } else if resp.signatures.is_empty() {
        "(no signatures registered)".to_string()
    } else {
        resp.signatures
            .iter()
            .map(|s| format!("{}  {}", hex::encode(&s.signature_id), s.name))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `signatures get` (the manifest is opaque bincode — hex, never decoded).
#[must_use]
pub fn render_signature_get(resp: &proto::GetSignatureResponse, json: bool) -> String {
    if json {
        json!({
            "signature_id": hex::encode(&resp.signature_id),
            "manifest_hex": hex::encode(&resp.manifest),
            "manifest_len": resp.manifest.len(),
        })
        .to_string()
    } else {
        format!(
            "signature_id {}\nmanifest     {} bytes (opaque)",
            hex::encode(&resp.signature_id),
            resp.manifest.len(),
        )
    }
}

/// Render `signatures register`.
#[must_use]
pub fn render_signature_register(resp: &proto::RegisterSignatureResponse, json: bool) -> String {
    if json {
        json!({ "signature_id": hex::encode(&resp.signature_id) }).to_string()
    } else {
        format!("signature_id {}", hex::encode(&resp.signature_id))
    }
}

/// Map a [`proto::LowerVerdict`] discriminant to a stable display name. An
/// out-of-range value renders `unknown` (forward-compatible). Matches the SDK
/// verdict names cross-surface.
#[must_use]
pub fn lower_verdict_name(verdict: i32) -> &'static str {
    use proto::LowerVerdict as V;
    if verdict == V::Unavailable as i32 {
        "unavailable"
    } else if verdict == V::WouldLower as i32 {
        "would-lower"
    } else if verdict == V::Refused as i32 {
        "refused"
    } else {
        "unknown"
    }
}

/// Render `tools list` — the registered manifests. ADVISORY discovery (SN-8):
/// listing a tool never grants it.
#[must_use]
pub fn render_tools_list(resp: &proto::ListToolManifestsResponse, json: bool) -> String {
    if json {
        let manifests: Vec<Value> = resp
            .manifests
            .iter()
            .map(|m| {
                json!({
                    "tool_id": m.tool_id,
                    "tool_version": m.tool_version,
                    "kind": m.kind,
                    "description": m.description,
                    "fingerprint_hash": hex::encode(&m.fingerprint_hash),
                    "keywords": m.keywords.iter().map(|k| json!({
                        "lang": k.lang,
                        "words": k.words,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({ "manifests": manifests }).to_string()
    } else if resp.manifests.is_empty() {
        "(no tools registered)".to_string()
    } else {
        resp.manifests
            .iter()
            .map(|m| {
                format!(
                    "{}@{}  [{}]  {}",
                    m.tool_id, m.tool_version, m.kind, m.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `tools discover` — the durable registry INVENTORY (PR-6a). Distinct
/// from `tools list` (advisory ranking): this is "what is registered, by whom,
/// with what authority". Registration grants no authority (SN-8).
#[must_use]
pub fn render_tools_discover(resp: &proto::DiscoverToolsResponse, json: bool) -> String {
    if json {
        let tools: Vec<Value> = resp
            .tools
            .iter()
            .map(|t| {
                json!({
                    "tool_id": hex::encode(&t.tool_id),
                    "tool_name": t.tool_name,
                    "tool_version": t.tool_version,
                    "kind": t.kind,
                    "description": t.description,
                    "idempotency_class": t.idempotency_class,
                    "provenance": t.provenance,
                    "registration_status": t.registration_status,
                    "server_host": t.server_host,
                    "net_scope": t.net_scope_summary,
                    "is_builtin": t.is_builtin,
                })
            })
            .collect();
        json!({ "tools": tools, "has_more": resp.has_more }).to_string()
    } else if resp.tools.is_empty() {
        "(no tools registered)".to_string()
    } else {
        resp.tools
            .iter()
            .map(|t| {
                let builtin = if t.is_builtin { " builtin" } else { "" };
                let host = if t.server_host.is_empty() {
                    String::new()
                } else {
                    format!("  ->{}", t.server_host)
                };
                format!(
                    "{}@{}  [{}{}]  {}{}  {}",
                    t.tool_name,
                    t.tool_version,
                    t.kind,
                    builtin,
                    t.net_scope_summary,
                    host,
                    t.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `tools register` — the SERVER-DERIVED tool_id + status.
#[must_use]
pub fn render_register_tool(resp: &proto::RegisterToolResponse, json: bool) -> String {
    if json {
        json!({
            "tool_id": hex::encode(&resp.tool_id),
            "registration_status": resp.registration_status,
        })
        .to_string()
    } else {
        format!(
            "registered tool_id={} ({})",
            hex::encode(&resp.tool_id),
            resp.registration_status
        )
    }
}

/// Render `tools deregister` — whether a row was removed.
#[must_use]
pub fn render_deregister_tool(resp: &proto::DeregisterToolResponse, json: bool) -> String {
    if json {
        json!({ "removed": resp.removed }).to_string()
    } else if resp.removed {
        "removed".to_string()
    } else {
        "not removed (absent or a built-in)".to_string()
    }
}

/// Render `connections add` — the server-derived connection id + discovery health.
#[must_use]
pub fn render_register_server(resp: &proto::RegisterMcpServerResponse, json: bool) -> String {
    if json {
        json!({
            "connection_id": hex::encode(&resp.connection_id),
            "discovered": resp.discovered,
            "health": resp.health,
        })
        .to_string()
    } else {
        format!(
            "registered connection_id={} ({}, {} tool(s) discovered)",
            hex::encode(&resp.connection_id),
            resp.health,
            resp.discovered
        )
    }
}

/// Render `connections list` — the registered external MCP servers + health.
#[must_use]
pub fn render_connections_list(resp: &proto::ListMcpServersResponse, json: bool) -> String {
    if json {
        let servers: Vec<Value> = resp
            .servers
            .iter()
            .map(|s| {
                json!({
                    "connection_id": hex::encode(&s.connection_id),
                    "server_name": s.server_name,
                    "transport": s.transport,
                    "endpoint": s.endpoint,
                    "health": s.health,
                    "tool_count": s.tool_count,
                    "credential_ref_present": s.credential_ref_present,
                    "session_mode": s.session_mode,
                })
            })
            .collect();
        json!({ "servers": servers, "has_more": resp.has_more }).to_string()
    } else if resp.servers.is_empty() {
        "(no MCP servers registered)".to_string()
    } else {
        resp.servers
            .iter()
            .map(|s| {
                let cred = if s.credential_ref_present {
                    "  cred"
                } else {
                    ""
                };
                // The session_mode is shown only when stateful (the non-default).
                let mode = if s.session_mode == "stateful" {
                    "  stateful"
                } else {
                    ""
                };
                format!(
                    "{}  [{}]  {}  {} tool(s)  ({}){}{}",
                    s.server_name, s.transport, s.endpoint, s.tool_count, s.health, cred, mode
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `connections test` — reachability + a short diagnostic.
#[must_use]
pub fn render_test_server(resp: &proto::TestMcpServerResponse, json: bool) -> String {
    if json {
        json!({ "reachable": resp.reachable, "detail": resp.detail }).to_string()
    } else if resp.reachable {
        format!("reachable ({})", resp.detail)
    } else {
        format!("unreachable ({})", resp.detail)
    }
}

/// Render `connections fire` — the diagnostic tool-fire result or the refusal.
#[must_use]
pub fn render_call_tool(resp: &proto::CallMcpToolResponse, json: bool) -> String {
    if json {
        json!({ "ok": resp.ok, "result": resp.result_json, "error": resp.error }).to_string()
    } else if resp.ok {
        format!("ok\n{}", resp.result_json)
    } else {
        format!("error: {}", resp.error)
    }
}

/// Render `connections remove` — whether the server was removed.
#[must_use]
pub fn render_deregister_server(resp: &proto::DeregisterMcpServerResponse, json: bool) -> String {
    if json {
        json!({ "removed": resp.removed }).to_string()
    } else if resp.removed {
        "removed".to_string()
    } else {
        "not removed (no such server)".to_string()
    }
}

/// Render `connections discover` — the server's registered tools (after re-dial).
#[must_use]
pub fn render_discover_server(resp: &proto::DiscoverServerToolsResponse, json: bool) -> String {
    if json {
        let tools: Vec<Value> = resp
            .tools
            .iter()
            .map(|t| {
                json!({
                    "tool_id": hex::encode(&t.tool_id),
                    "tool_name": t.tool_name,
                    "tool_version": t.tool_version,
                    "kind": t.kind,
                    "description": t.description,
                    "net_scope": t.net_scope_summary,
                })
            })
            .collect();
        json!({ "tools": tools, "discovered": resp.discovered }).to_string()
    } else if resp.tools.is_empty() {
        format!("({} tool(s) discovered; none registered)", resp.discovered)
    } else {
        let header = format!("{} tool(s) discovered:", resp.discovered);
        let rows = resp
            .tools
            .iter()
            .map(|t| {
                format!(
                    "  {}@{}  [{}]  {}  {}",
                    t.tool_name, t.tool_version, t.kind, t.net_scope_summary, t.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("{header}\n{rows}")
    }
}

// --- PR-7 context bundles --------------------------------------------------

/// Render the displayed items of a context bundle (shared by get/list JSON).
fn context_items_json(items: &[proto::ContextItem]) -> Vec<Value> {
    items
        .iter()
        .map(|it| {
            json!({
                "name": it.name,
                "content_ref": hex::encode(&it.content_ref),
                "media_type": it.media_type,
            })
        })
        .collect()
}

/// Render `context add` — the server-derived bundle ref + dedup signal.
#[must_use]
pub fn render_put_context_bundle(resp: &proto::PutContextBundleResponse, json: bool) -> String {
    if json {
        json!({
            "bundle_ref": hex::encode(&resp.bundle_ref),
            "handle": resp.handle,
            "deduplicated": resp.deduplicated,
        })
        .to_string()
    } else {
        format!(
            "bundle {} ref={} deduplicated={}",
            resp.handle,
            hex::encode(&resp.bundle_ref),
            resp.deduplicated
        )
    }
}

/// Render `context list` — the caller's bundles in handle order.
#[must_use]
pub fn render_context_bundles_list(resp: &proto::ListContextBundlesResponse, json: bool) -> String {
    if json {
        let bundles: Vec<Value> = resp
            .bundles
            .iter()
            .map(|b| {
                json!({
                    "bundle_ref": hex::encode(&b.bundle_ref),
                    "handle": b.handle,
                    "description": b.description,
                    "item_count": b.item_count,
                    "items": context_items_json(&b.items),
                })
            })
            .collect();
        json!({ "bundles": bundles, "has_more": resp.has_more }).to_string()
    } else if resp.bundles.is_empty() {
        "(no context bundles)".to_string()
    } else {
        resp.bundles
            .iter()
            .map(|b| format!("{}  {} item(s)  {}", b.handle, b.item_count, b.description))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `context get` — one bundle's manifest (uniform not-found, no oracle).
#[must_use]
pub fn render_get_context_bundle(resp: &proto::GetContextBundleResponse, json: bool) -> String {
    let Some(b) = resp.bundle.as_ref().filter(|_| resp.found) else {
        return if json {
            json!({ "found": false }).to_string()
        } else {
            "(not found)".to_string()
        };
    };
    if json {
        json!({
            "found": true,
            "bundle_ref": hex::encode(&b.bundle_ref),
            "handle": b.handle,
            "description": b.description,
            "item_count": b.item_count,
            "items": context_items_json(&b.items),
        })
        .to_string()
    } else {
        let header = format!(
            "{}  ref={}  {} item(s)  {}",
            b.handle,
            hex::encode(&b.bundle_ref),
            b.item_count,
            b.description
        );
        let rows = b
            .items
            .iter()
            .map(|it| format!("  {} -> {}", it.name, hex::encode(&it.content_ref)))
            .collect::<Vec<_>>()
            .join("\n");
        if rows.is_empty() {
            header
        } else {
            format!("{header}\n{rows}")
        }
    }
}

/// Render `context remove` — whether the bundle was unbound.
#[must_use]
pub fn render_delete_context_bundle(
    resp: &proto::DeleteContextBundleResponse,
    json: bool,
) -> String {
    if json {
        json!({ "removed": resp.removed }).to_string()
    } else if resp.removed {
        "removed".to_string()
    } else {
        "not removed (no such bundle)".to_string()
    }
}

// ----- POC-4 Apps -----

/// Render `app save` — the server-derived `app_ref` + dedup signal.
#[must_use]
pub fn render_save_app(resp: &proto::SaveAppResponse, json: bool) -> String {
    if json {
        json!({
            "app_ref": hex::encode(&resp.app_ref),
            "handle": resp.handle,
            "deduplicated": resp.deduplicated,
        })
        .to_string()
    } else {
        format!(
            "app {} ref={} deduplicated={}",
            resp.handle,
            hex::encode(&resp.app_ref),
            resp.deduplicated
        )
    }
}

/// JSON of one App summary (the catalog/display view).
fn app_summary_json(s: &proto::AppSummary) -> Value {
    json!({
        "handle": s.handle,
        "app_ref": hex::encode(&s.app_ref),
        "name": s.name,
        "version": s.version,
        "description": s.description,
        "tags": s.tags,
        "step_count": s.step_count,
        "locked": s.locked,
    })
}

/// Render `app scaffold` — the launched server-side scaffold (correlate by branch).
#[must_use]
pub fn render_scaffold_app(resp: &proto::ScaffoldAppResponse, json: bool) -> String {
    if json {
        json!({ "branch_handle": resp.branch_handle, "resumed": resp.resumed }).to_string()
    } else {
        format!(
            "scaffolding into branch {} (resumed={}) — poll `kx app files {}` / `kx app scaffold-status`",
            resp.branch_handle, resp.resumed, resp.branch_handle
        )
    }
}

/// Render the scaffold status (phase + the done/pending skeleton files).
#[must_use]
pub fn render_scaffold_status(resp: &proto::GetScaffoldStatusResponse, json: bool) -> String {
    let phase = match proto::get_scaffold_status_response::Phase::try_from(resp.phase) {
        Ok(proto::get_scaffold_status_response::Phase::Planning) => "planning",
        Ok(proto::get_scaffold_status_response::Phase::Writing) => "writing",
        Ok(proto::get_scaffold_status_response::Phase::Done) => "done",
        Ok(proto::get_scaffold_status_response::Phase::Failed) => "failed",
        _ => "unspecified",
    };
    if json {
        json!({
            "phase": phase,
            "files_done": resp.files_done,
            "files_pending": resp.files_pending,
            "detail": resp.detail,
        })
        .to_string()
    } else {
        use std::fmt::Write as _;
        let mut s = format!(
            "scaffold {phase} — {} done, {} pending",
            resp.files_done.len(),
            resp.files_pending.len()
        );
        for f in &resp.files_done {
            let _ = write!(s, "\n  ✓ {f}");
        }
        for f in &resp.files_pending {
            let _ = write!(s, "\n  · {f}");
        }
        if !resp.detail.is_empty() {
            let _ = write!(s, "\n  ({})", resp.detail);
        }
        s
    }
}

/// Render `app lock` / `app unlock` — the per-App branch lock state.
#[must_use]
pub fn render_app_lock(handle: &str, locked: bool, json: bool) -> String {
    if json {
        json!({ "branch_handle": handle, "locked": locked }).to_string()
    } else {
        format!(
            "app {handle} {}",
            if locked { "locked" } else { "unlocked" }
        )
    }
}

/// Render `app list` — the caller's App catalog (deterministic handle order).
#[must_use]
pub fn render_apps_list(resp: &proto::ListAppsResponse, json: bool) -> String {
    if json {
        let apps: Vec<Value> = resp.apps.iter().map(app_summary_json).collect();
        json!({ "apps": apps, "has_more": resp.has_more }).to_string()
    } else if resp.apps.is_empty() {
        "(no apps)".to_string()
    } else {
        resp.apps
            .iter()
            .map(|a| {
                format!(
                    "{}  {} v{}  {} step(s)  {}",
                    a.handle, a.name, a.version, a.step_count, a.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `app get` — one App's summary (uniform not-found, no oracle). The
/// envelope bytes are written to `--output` by the verb, not printed here.
#[must_use]
pub fn render_get_app(resp: &proto::GetAppResponse, json: bool) -> String {
    let Some(s) = resp.summary.as_ref().filter(|_| resp.found) else {
        return if json {
            json!({ "found": false }).to_string()
        } else {
            "(not found)".to_string()
        };
    };
    if json {
        json!({ "found": true, "summary": app_summary_json(s) }).to_string()
    } else {
        let tags = if s.tags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", s.tags.join(", "))
        };
        format!(
            "{}  {} v{}  ref={}  {} step(s){}\n  {}",
            s.handle,
            s.name,
            s.version,
            hex::encode(&s.app_ref),
            s.step_count,
            tags,
            s.description
        )
    }
}

// ----- D155 branches -----

/// JSON of a branch manifest's `{path -> ref}` items (path-sorted display).
fn branch_items_json(items: &[proto::BranchItem]) -> Vec<Value> {
    items
        .iter()
        .map(|it| json!({ "path": it.path, "content_ref": hex::encode(&it.content_ref) }))
        .collect()
}

/// Render `branch create` — the new/forked branch ref.
#[must_use]
pub fn render_create_branch(resp: &proto::CreateBranchResponse, json: bool) -> String {
    if json {
        json!({
            "branch_ref": hex::encode(&resp.branch_ref),
            "handle": resp.handle,
            "deduplicated": resp.deduplicated,
        })
        .to_string()
    } else {
        format!(
            "branch {} ref={} deduplicated={}",
            resp.handle,
            hex::encode(&resp.branch_ref),
            resp.deduplicated
        )
    }
}

/// Render `branch snapshot` — the resolved manifest + how many paths were ingested.
#[must_use]
pub fn render_snapshot_into(resp: &proto::SnapshotIntoResponse, json: bool) -> String {
    if json {
        json!({
            "branch_ref": hex::encode(&resp.branch_ref),
            "handle": resp.handle,
            "ingested": resp.ingested,
            "item_count": resp.items.len(),
            "items": branch_items_json(&resp.items),
            "deduplicated": resp.deduplicated,
        })
        .to_string()
    } else {
        let header = format!(
            "branch {} ref={}  ingested={}  {} file(s)",
            resp.handle,
            hex::encode(&resp.branch_ref),
            resp.ingested,
            resp.items.len()
        );
        let rows = resp
            .items
            .iter()
            .map(|it| format!("  {} -> {}", it.path, hex::encode(&it.content_ref)))
            .collect::<Vec<_>>()
            .join("\n");
        if rows.is_empty() {
            header
        } else {
            format!("{header}\n{rows}")
        }
    }
}

/// Render `branch advance` / the post-`edit` manifest — the re-pointed manifest
/// (D155 Phase-3). Mirrors `render_snapshot_into` minus the `ingested` count.
#[must_use]
pub fn render_advance_branch(resp: &proto::AdvanceBranchResponse, json: bool) -> String {
    if json {
        json!({
            "branch_ref": hex::encode(&resp.branch_ref),
            "handle": resp.handle,
            "item_count": resp.items.len(),
            "items": branch_items_json(&resp.items),
            "deduplicated": resp.deduplicated,
        })
        .to_string()
    } else {
        let header = format!(
            "branch {} advanced ref={}  {} file(s){}",
            resp.handle,
            hex::encode(&resp.branch_ref),
            resp.items.len(),
            if resp.deduplicated {
                "  (no change)"
            } else {
                ""
            }
        );
        let rows = resp
            .items
            .iter()
            .map(|it| format!("  {} -> {}", it.path, hex::encode(&it.content_ref)))
            .collect::<Vec<_>>()
            .join("\n");
        if rows.is_empty() {
            header
        } else {
            format!("{header}\n{rows}")
        }
    }
}

/// Render `branch list` — the caller's branches in handle order.
#[must_use]
pub fn render_branches_list(resp: &proto::ListBranchesResponse, json: bool) -> String {
    if json {
        let branches: Vec<Value> = resp
            .branches
            .iter()
            .map(|b| {
                json!({
                    "branch_ref": hex::encode(&b.branch_ref),
                    "handle": b.handle,
                    "parent_handle": b.parent_handle,
                    "description": b.description,
                    "item_count": b.item_count,
                })
            })
            .collect();
        json!({ "branches": branches, "has_more": resp.has_more }).to_string()
    } else if resp.branches.is_empty() {
        "(no branches)".to_string()
    } else {
        resp.branches
            .iter()
            .map(|b| {
                let parent = if b.parent_handle.is_empty() {
                    String::new()
                } else {
                    format!("  <- {}", b.parent_handle)
                };
                format!(
                    "{}  {} file(s){}  {}",
                    b.handle, b.item_count, parent, b.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `branch get` — one branch's resolved manifest (uniform not-found).
#[must_use]
pub fn render_get_branch(resp: &proto::GetBranchResponse, json: bool) -> String {
    let Some(b) = resp.branch.as_ref().filter(|_| resp.found) else {
        return if json {
            json!({ "found": false }).to_string()
        } else {
            "(not found)".to_string()
        };
    };
    if json {
        json!({
            "found": true,
            "branch_ref": hex::encode(&b.branch_ref),
            "handle": b.handle,
            "parent_handle": b.parent_handle,
            "description": b.description,
            "item_count": b.item_count,
            "items": branch_items_json(&b.items),
        })
        .to_string()
    } else {
        let parent = if b.parent_handle.is_empty() {
            String::new()
        } else {
            format!("  parent={}", b.parent_handle)
        };
        let header = format!(
            "{}  ref={}{}  {} file(s)  {}",
            b.handle,
            hex::encode(&b.branch_ref),
            parent,
            b.item_count,
            b.description
        );
        let rows = b
            .items
            .iter()
            .map(|it| format!("  {} -> {}", it.path, hex::encode(&it.content_ref)))
            .collect::<Vec<_>>()
            .join("\n");
        if rows.is_empty() {
            header
        } else {
            format!("{header}\n{rows}")
        }
    }
}

/// Render `branch remove` — whether the branch was unbound.
#[must_use]
pub fn render_delete_branch(resp: &proto::DeleteBranchResponse, json: bool) -> String {
    if json {
        json!({ "removed": resp.removed }).to_string()
    } else if resp.removed {
        "removed".to_string()
    } else {
        "not removed (no such branch)".to_string()
    }
}

/// Render `tools score` — the advisory rank ladder + the lowering dry-run
/// verdict. Every number is DISPLAY-ONLY (SN-8): a score can surface a tool,
/// never grant one.
#[must_use]
pub fn render_tools_score(resp: &proto::ScoreTaskBundleResponse, json: bool) -> String {
    let verdict = lower_verdict_name(resp.verdict);
    if json {
        let ranked: Vec<Value> = resp
            .ranked
            .iter()
            .map(|r| {
                json!({
                    "tool_id": r.tool_id,
                    "tool_version": r.tool_version,
                    "score_bp": r.score_bp,
                    "fingerprint_hash": hex::encode(&r.fingerprint_hash),
                })
            })
            .collect();
        json!({
            "bundle_fingerprint": hex::encode(&resp.bundle_fingerprint),
            "ranked": ranked,
            "verdict": verdict,
            "verdict_detail": resp.verdict_detail,
            "advisory": "scores never authorize a tool",
        })
        .to_string()
    } else {
        let mut out = format!(
            "bundle           {}\n",
            hex::encode(&resp.bundle_fingerprint)
        );
        let _ = writeln!(out, "verdict          {verdict}");
        if !resp.verdict_detail.is_empty() {
            let _ = writeln!(out, "                 ({})", resp.verdict_detail);
        }
        out.push_str("ranked (advisory — scores never authorize):");
        for r in &resp.ranked {
            // Only the 10000 ceiling proves an exact keyword/phrase hit; the
            // sub-ceiling fuzzy/vector bands overlap on the wire, so anything
            // else is honestly just "similar".
            let rung = if r.score_bp == 10_000 {
                "exact"
            } else if r.score_bp > 0 {
                "similar"
            } else {
                "-"
            };
            let _ = write!(
                out,
                "\n  {:>5} bp  {:<8} {}@{}",
                r.score_bp, rung, r.tool_id, r.tool_version
            );
        }
        out
    }
}

/// Render `ListRecipes` — the provisioned recipe handles + their advisory
/// metadata (PR-4 Batch D). Display-only; `kx invoke` stays the gate.
pub fn render_recipes_list(resp: &proto::ListRecipesResponse, json: bool) -> String {
    if json {
        let recipes: Vec<Value> = resp
            .recipes
            .iter()
            .map(|r| {
                json!({
                    "handle": r.handle,
                    "recipe_fingerprint": hex::encode(&r.recipe_fingerprint),
                    "description": r.description,
                    "tags": r.tags,
                    "version": r.version,
                })
            })
            .collect();
        json!({ "recipes": recipes }).to_string()
    } else if resp.recipes.is_empty() {
        "(no recipes provisioned)".to_string()
    } else {
        resp.recipes
            .iter()
            .map(|r| {
                let tags = if r.tags.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", r.tags.join(", "))
                };
                format!("{}{}  {}", r.handle, tags, r.description)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `SearchRecipes` — the advisory ranking (PR-4 Batch D). `score_bp` is
/// display-only basis points; a hit SURFACES a recipe, never invokes one.
pub fn render_recipes_search(resp: &proto::SearchRecipesResponse, json: bool) -> String {
    if json {
        let ranked: Vec<Value> = resp
            .ranked
            .iter()
            .map(|s| {
                let r = s.recipe.as_ref();
                json!({
                    "handle": r.map(|r| r.handle.as_str()).unwrap_or_default(),
                    "score_bp": s.score_bp,
                    "description": r.map(|r| r.description.as_str()).unwrap_or_default(),
                    "tags": r.map(|r| r.tags.clone()).unwrap_or_default(),
                    "version": r.map(|r| r.version.as_str()).unwrap_or_default(),
                })
            })
            .collect();
        json!({ "ranked": ranked, "advisory": "scores never authorize a recipe" }).to_string()
    } else if resp.ranked.is_empty() {
        "(no matching recipes)".to_string()
    } else {
        let mut out = String::from("ranked (advisory — scores never authorize):");
        for s in &resp.ranked {
            let handle = s.recipe.as_ref().map_or("?", |r| r.handle.as_str());
            let rung = if s.score_bp == 10_000 {
                "exact"
            } else if s.score_bp > 0 {
                "match"
            } else {
                "-"
            };
            let _ = write!(out, "\n  {:>5} bp  {:<6} {}", s.score_bp, rung, handle);
        }
        out
    }
}

/// Map a [`proto::NdClass`] discriminant to a stable display name.
#[must_use]
pub fn nd_class_name(nd: i32) -> &'static str {
    use proto::NdClass as N;
    if nd == N::Pure as i32 {
        "PURE"
    } else if nd == N::ReadOnlyNondet as i32 {
        "READ_ONLY_NONDET"
    } else if nd == N::WorldMutating as i32 {
        "WORLD_MUTATING"
    } else {
        "UNKNOWN"
    }
}

/// Map a [`proto::NdClass`] discriminant to the stable lowercase wire tag the
/// WS/SDK surfaces speak (`"pure"` / `"read_only_nondet"` / `"world_mutating"`;
/// out-of-range → `"unspecified"`). Distinct from the uppercase display
/// [`nd_class_name`] — the global event tail's `--json` parity needs this form.
#[must_use]
pub fn nd_class_tag(nd: i32) -> &'static str {
    use proto::NdClass as N;
    if nd == N::Pure as i32 {
        "pure"
    } else if nd == N::ReadOnlyNondet as i32 {
        "read_only_nondet"
    } else if nd == N::WorldMutating as i32 {
        "world_mutating"
    } else {
        "unspecified"
    }
}

/// Map a [`proto::EffectPattern`] discriminant to a stable display name.
#[must_use]
pub fn effect_pattern_name(ep: i32) -> &'static str {
    use proto::EffectPattern as E;
    if ep == E::IdempotentByConstruction as i32 {
        "IdempotentByConstruction"
    } else if ep == E::StageThenCommit as i32 {
        "StageThenCommit"
    } else if ep == E::ValidateThenCommit as i32 {
        "ValidateThenCommit"
    } else {
        "UNKNOWN"
    }
}

/// Render `runs list` (Batch B): newest-first run summaries + the pagination
/// cursor hint. `--json` field names mirror the TS `RunSummary.toJSON` /
/// Py `to_dict` snake_case shape (the tri-surface parity contract).
#[must_use]
pub fn render_runs(resp: &proto::ListRunsResponse, json: bool) -> String {
    if json {
        let runs: Vec<Value> = resp
            .runs
            .iter()
            .map(|r| {
                json!({
                    "instance_id": hex::encode(&r.instance_id),
                    "recipe_fingerprint": hex::encode(&r.recipe_fingerprint),
                    "registered_seq": r.registered_seq,
                    "registered_unix_ms": r.registered_unix_ms,
                })
            })
            .collect();
        json!({ "runs": runs, "has_more": resp.has_more }).to_string()
    } else if resp.runs.is_empty() {
        "(no runs)".to_string()
    } else {
        let mut out = String::new();
        for r in &resp.runs {
            let _ = write!(
                out,
                "{}{}  recipe {}  seq {}  registered_ms {}",
                if out.is_empty() { "" } else { "\n" },
                hex::encode(&r.instance_id),
                hex::encode(&r.recipe_fingerprint),
                r.registered_seq,
                r.registered_unix_ms,
            );
        }
        if resp.has_more {
            let last = resp.runs.last().map_or(0, |r| r.registered_seq);
            let _ = write!(out, "\n(more — continue with --before-seq {last})");
        }
        out
    }
}

/// Render `mote show` (Batch B): the capped, display-only definition summary.
/// `--json` field names mirror the TS `MoteDetail.toJSON` / Py `to_dict`
/// snake_case shape (the tri-surface parity contract).
#[must_use]
pub fn render_mote_detail(detail: &proto::MoteDetail, json: bool) -> String {
    if json {
        let config: Vec<Value> = detail
            .config_subset
            .iter()
            .map(|e| {
                json!({
                    "key": e.key,
                    "value_hex": hex::encode(&e.value),
                    "truncated": e.truncated,
                    "full_len": e.full_len,
                })
            })
            .collect();
        let tools: std::collections::BTreeMap<_, _> = detail.tool_contract.iter().collect();
        json!({
            "mote_id": hex::encode(&detail.mote_id),
            "mote_def_hash": hex::encode(&detail.mote_def_hash),
            "def_found": detail.def_found,
            "step_kind": detail.step_kind,
            "model_id": detail.model_id,
            "prompt": detail.prompt,
            "prompt_truncated": detail.prompt_truncated,
            "config_subset": config,
            "tool_contract": tools,
            "logic_ref": hex::encode(&detail.logic_ref),
            "nd_class": nd_class_name(detail.nd_class),
            "effect_pattern": effect_pattern_name(detail.effect_pattern),
            "critic_for": detail.critic_for.as_deref().map(hex::encode),
            "is_topology_shaper": detail.is_topology_shaper,
            "schema_version": detail.schema_version,
        })
        .to_string()
    } else if !detail.def_found {
        format!(
            "mote {}\n  def_found: false{}",
            hex::encode(&detail.mote_id),
            if detail.mote_def_hash.is_empty() {
                "  (not committed yet — the def hash only exists on a Committed fact)"
            } else {
                "  (definition not retained — admitted by a pre-Batch-B binary)"
            }
        )
    } else {
        let mut out = format!(
            "mote {}\n  def      {}\n  kind     {}\n  nd       {}\n  effect   {}",
            hex::encode(&detail.mote_id),
            hex::encode(&detail.mote_def_hash),
            detail.step_kind,
            nd_class_name(detail.nd_class),
            effect_pattern_name(detail.effect_pattern),
        );
        if !detail.model_id.is_empty() {
            let _ = write!(out, "\n  model    {}", detail.model_id);
        }
        let _ = write!(out, "\n  logic    {}", hex::encode(&detail.logic_ref));
        if detail.is_topology_shaper {
            out.push_str("\n  shaper   true");
        }
        if let Some(target) = detail.critic_for.as_deref() {
            let _ = write!(out, "\n  critic_for {}", hex::encode(target));
        }
        if !detail.tool_contract.is_empty() {
            let tools: std::collections::BTreeMap<_, _> = detail.tool_contract.iter().collect();
            let listed = tools
                .iter()
                .map(|(name, version)| format!("{name}@{version}"))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(out, "\n  tools    {listed}");
        }
        if !detail.prompt.is_empty() {
            let _ = write!(
                out,
                "\n  prompt{}:\n    {}",
                if detail.prompt_truncated {
                    " (truncated)"
                } else {
                    ""
                },
                detail.prompt.replace('\n', "\n    "),
            );
        }
        for e in &detail.config_subset {
            let shown = String::from_utf8(e.value.clone())
                .unwrap_or_else(|_| format!("0x{}", hex::encode(&e.value)));
            let _ = write!(
                out,
                "\n  param    {} = {}{}",
                e.key,
                shown,
                if e.truncated {
                    format!(" (+{} bytes truncated)", e.full_len - e.value.len() as u64)
                } else {
                    String::new()
                }
            );
        }
        out
    }
}

/// Hex of a telemetry attribution id; the ALL-ZERO (or empty) unattributed
/// sentinel renders as the empty string (wire parity with the WS/SDK).
fn telemetry_instance_hex(id: &[u8]) -> String {
    if id.iter().all(|&b| b == 0) {
        String::new()
    } else {
        hex::encode(id)
    }
}

/// Render `telemetry list` (Batch C): newest-first mote execution exhaust +
/// the pagination cursor hint. `--json` field names mirror the SDK snake_case
/// shape (the tri-surface parity contract); absent token counts are `null`
/// (`input_tokens` is never set in OSS).
#[must_use]
pub fn render_telemetry(resp: &proto::ListMoteTelemetryResponse, json: bool) -> String {
    if json {
        let rows: Vec<Value> = resp
            .rows
            .iter()
            .map(|r| {
                json!({
                    "mote_id": hex::encode(&r.mote_id),
                    "instance_id": telemetry_instance_hex(&r.instance_id),
                    "wall_clock_ms": r.wall_clock_ms,
                    "input_tokens": r.input_tokens,
                    "output_tokens": r.output_tokens,
                    "model_id": r.model_id,
                    "tool_id": r.tool_id,
                    "started_unix_ms": r.started_unix_ms,
                    "seq": r.seq,
                })
            })
            .collect();
        json!({ "rows": rows, "has_more": resp.has_more }).to_string()
    } else if resp.rows.is_empty() {
        "(no telemetry rows)".to_string()
    } else {
        let dash = |s: &str| {
            if s.is_empty() {
                "-".to_string()
            } else {
                s.to_string()
            }
        };
        let opt = |v: Option<u64>| v.map_or_else(|| "-".to_string(), |n| n.to_string());
        let mut out = String::new();
        for r in &resp.rows {
            let _ = write!(
                out,
                "{}{}  inst {}  {}ms  model {}  tool {}  tokens {}/{}  started_ms {}  seq {}",
                if out.is_empty() { "" } else { "\n" },
                hex::encode(&r.mote_id),
                dash(&telemetry_instance_hex(&r.instance_id)),
                r.wall_clock_ms,
                dash(&r.model_id),
                dash(&r.tool_id),
                opt(r.input_tokens),
                opt(r.output_tokens),
                r.started_unix_ms,
                r.seq,
            );
        }
        if resp.has_more {
            let last = resp.rows.last().map_or(0, |r| r.seq);
            let _ = write!(out, "\n(more — continue with --before-seq {last})");
        }
        out
    }
}

/// Render `telemetry summary` (W1a-3): the exact, cross-page per-model
/// token-economy rollup + the window-wide totals. The empty state is honest
/// ("(no telemetry rows)"), never a fabricated row; no cost/$ (billing is
/// CLOUD). `--json` field names mirror the SDK snake_case shape (the
/// tri-surface parity contract).
#[must_use]
pub fn render_telemetry_summary(resp: &proto::ListTelemetrySummaryResponse, json: bool) -> String {
    if json {
        let rows: Vec<Value> = resp
            .rows
            .iter()
            .map(|r| {
                json!({
                    "model_id": r.model_id,
                    "count": r.count,
                    "total_output_tokens": r.total_output_tokens,
                    "total_wall_clock_ms": r.total_wall_clock_ms,
                })
            })
            .collect();
        json!({
            "rows": rows,
            "total_motes": resp.total_motes,
            "total_output_tokens": resp.total_output_tokens,
        })
        .to_string()
    } else if resp.rows.is_empty() && resp.total_motes == 0 {
        "(no telemetry rows)".to_string()
    } else {
        let mut out = String::new();
        for r in &resp.rows {
            let _ = write!(
                out,
                "{}model {}  motes {}  out_tokens {}  wall_ms {}",
                if out.is_empty() { "" } else { "\n" },
                r.model_id,
                r.count,
                r.total_output_tokens,
                r.total_wall_clock_ms,
            );
        }
        let _ = write!(
            out,
            "{}total: {} motes, {} output tokens",
            if out.is_empty() { "" } else { "\n" },
            resp.total_motes,
            resp.total_output_tokens,
        );
        out
    }
}

/// Render `alerts list` (W1a-2): newest-first terminal-failure alerts + the
/// pagination cursor hint. The empty state is honest ("System is healthy …"),
/// not a fabricated row. `--json` field names mirror the SDK snake_case shape
/// (the tri-surface parity contract).
#[must_use]
pub fn render_alerts(resp: &proto::ListAlertsResponse, json: bool) -> String {
    if json {
        let rows: Vec<Value> = resp
            .alerts
            .iter()
            .map(|a| {
                json!({
                    "alert_id": hex::encode(&a.alert_id),
                    "mote_id": hex::encode(&a.mote_id),
                    "instance_id": telemetry_instance_hex(&a.instance_id),
                    "reason_class": a.reason_class,
                    "reason_code": a.reason_code,
                    "severity": a.severity,
                    "seq": a.seq,
                    "created_unix_ms": a.created_unix_ms,
                })
            })
            .collect();
        json!({ "alerts": rows, "has_more": resp.has_more }).to_string()
    } else if resp.alerts.is_empty() {
        "System is healthy — no terminal failures or refusals.".to_string()
    } else {
        let dash = |s: &str| {
            if s.is_empty() {
                "-".to_string()
            } else {
                s.to_string()
            }
        };
        let mut out = String::new();
        for a in &resp.alerts {
            let _ = write!(
                out,
                "{}[{}] {}  mote {}  inst {}  seq {}",
                if out.is_empty() { "" } else { "\n" },
                a.severity,
                a.reason_class,
                hex::encode(&a.mote_id),
                dash(&telemetry_instance_hex(&a.instance_id)),
                a.seq,
            );
        }
        if resp.has_more {
            let last = resp.alerts.last().map_or(0, |a| a.seq);
            let _ = write!(out, "\n(more — continue with --before-seq {last})");
        }
        out
    }
}

/// Render `feedback submit` (PR-4.1): the server-derived `feedback_id`.
#[must_use]
pub fn render_feedback_submit(resp: &proto::SubmitFeedbackResponse, json: bool) -> String {
    let id = hex::encode(&resp.feedback_id);
    if json {
        json!({ "feedback_id": id }).to_string()
    } else {
        format!("recorded feedback {id}")
    }
}

/// Render `feedback list` (PR-4.1): newest-first 👍/👎 rows. `--json` field names
/// mirror the SDK snake_case shape; byte ids are hex (empty targets → "").
#[must_use]
pub fn render_feedback_list(resp: &proto::ListFeedbackResponse, json: bool) -> String {
    let rating_str = |r: i32| -> &'static str {
        if r == proto::FeedbackRating::Up as i32 {
            "up"
        } else if r == proto::FeedbackRating::Down as i32 {
            "down"
        } else {
            "?"
        }
    };
    // An all-zero / empty target id renders as "" (the telemetry instance convention).
    let opt_hex = |b: &[u8]| -> String {
        if b.iter().all(|x| *x == 0) {
            String::new()
        } else {
            hex::encode(b)
        }
    };
    if json {
        let rows: Vec<Value> = resp
            .rows
            .iter()
            .map(|r| {
                json!({
                    "feedback_id": hex::encode(&r.feedback_id),
                    "rating": rating_str(r.rating),
                    "message_id": r.message_id,
                    "instance_id": opt_hex(&r.instance_id),
                    "mote_id": opt_hex(&r.mote_id),
                    "content_ref": opt_hex(&r.content_ref),
                    "comment": r.comment,
                    "recipe_handle": r.recipe_handle,
                    "model_id": r.model_id,
                    "submitted_unix_ms": r.submitted_unix_ms,
                    "rowid": r.rowid,
                })
            })
            .collect();
        json!({ "rows": rows, "has_more": resp.has_more }).to_string()
    } else if resp.rows.is_empty() {
        "(no feedback rows)".to_string()
    } else {
        let dash = |s: &str| {
            if s.is_empty() {
                "-".to_string()
            } else {
                s.to_string()
            }
        };
        let mut out = String::new();
        for r in &resp.rows {
            let inst = opt_hex(&r.instance_id);
            let comment = if r.comment.is_empty() {
                String::new()
            } else {
                format!("\"{}\"  ", r.comment)
            };
            let _ = write!(
                out,
                "{}{}  msg {}  inst {}  model {}  {}{}  rowid {}",
                if out.is_empty() { "" } else { "\n" },
                rating_str(r.rating),
                r.message_id,
                dash(&inst),
                dash(&r.model_id),
                comment,
                dash(&r.recipe_handle),
                r.rowid,
            );
        }
        if resp.has_more {
            let last = resp.rows.last().map_or(0, |r| r.rowid);
            let _ = write!(out, "\n(more — continue with --before-rowid {last})");
        }
        out
    }
}

/// Render `replan list` (PR-2c-2 observability): newest-first re-plan rounds.
/// `--json` field names mirror the SDK snake_case shape; byte ids are hex.
#[must_use]
pub fn render_replan_rounds(resp: &proto::ListReplanRoundsResponse, json: bool) -> String {
    if json {
        let rounds: Vec<Value> = resp
            .rounds
            .iter()
            .map(|r| {
                json!({
                    "round": r.round,
                    "shaper_mote_id": hex::encode(&r.shaper_mote_id),
                    "model_id": r.model_id,
                    "failed_step_ids": r.failed_step_ids.iter().map(|s| hex::encode(s)).collect::<Vec<_>>(),
                    "escalated": r.escalated,
                    "seq": r.seq,
                })
            })
            .collect();
        json!({ "rounds": rounds, "has_more": resp.has_more }).to_string()
    } else if resp.rounds.is_empty() {
        "(no replan rounds)".to_string()
    } else {
        let mut out = String::new();
        for r in &resp.rounds {
            let failed = if r.failed_step_ids.is_empty() {
                "-".to_string()
            } else {
                r.failed_step_ids
                    .iter()
                    .map(|s| hex::encode(s))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let _ = write!(
                out,
                "{}round {}  shaper {}  model {}  failed {}  escalated={}  seq {}",
                if out.is_empty() { "" } else { "\n" },
                r.round,
                hex::encode(&r.shaper_mote_id),
                r.model_id,
                failed,
                r.escalated,
                r.seq,
            );
        }
        if resp.has_more {
            out.push_str("\n(more — raise --limit)");
        }
        out
    }
}

/// Render `react list` (PR-2d-1 observability): newest-first ReAct turns.
/// `--json` field names mirror the SDK snake_case shape; byte ids are hex.
#[must_use]
pub fn render_react_turns(resp: &proto::ListReactTurnsResponse, json: bool) -> String {
    if json {
        let turns: Vec<Value> = resp
            .turns
            .iter()
            .map(|t| {
                json!({
                    "turn": t.turn,
                    "turn_mote_id": hex::encode(&t.turn_mote_id),
                    "instance_id": hex::encode(&t.instance_id),
                    "model_id": t.model_id,
                    "branch": t.branch,
                    "tool_id": t.tool_id,
                    "tool_version": t.tool_version,
                    "max_turns": t.max_turns,
                    "max_tool_calls": t.max_tool_calls,
                    "seq": t.seq,
                    "rejection_reason": t.rejection_reason,
                    "call_index": t.call_index,
                })
            })
            .collect();
        json!({ "turns": turns, "has_more": resp.has_more }).to_string()
    } else if resp.turns.is_empty() {
        "(no react turns)".to_string()
    } else {
        let mut out = String::new();
        for t in &resp.turns {
            let detail = if !t.tool_id.is_empty() {
                // T-MULTI-ELEMENT-TOOLCALLS: show the call_index so a multi-tool turn's
                // parallel calls read `tool[0]` / `tool[1]`.
                format!(" tool[{}] {}@{}", t.call_index, t.tool_id, t.tool_version)
            } else if !t.rejection_reason.is_empty() {
                // PR-3 (A2): show WHY a turn was rejected so an operator can see
                // the model self-correct (or, at budget exhaustion, why it died).
                format!(" reason {}", t.rejection_reason)
            } else {
                String::new()
            };
            let _ = write!(
                out,
                "{}turn {}  inst {}  branch {}{}  model {}  caps {}/{}  seq {}",
                if out.is_empty() { "" } else { "\n" },
                t.turn,
                hex::encode(&t.instance_id),
                t.branch,
                detail,
                t.model_id,
                t.max_turns,
                t.max_tool_calls,
                t.seq,
            );
        }
        if resp.has_more {
            out.push_str("\n(more — raise --limit)");
        }
        out
    }
}

/// Render `capture list` (the Morphic Data Engine read surface): newest-first
/// captured-action join keys. `--json` field names mirror the SDK snake_case
/// shape; an absent `react_turn` is `null` (the Mote is not a ReAct turn).
#[must_use]
pub fn render_capture_records(resp: &proto::ListCaptureRecordsResponse, json: bool) -> String {
    if json {
        let records: Vec<Value> = resp
            .records
            .iter()
            .map(|r| {
                json!({
                    "mote_id": hex::encode(&r.mote_id),
                    "instance_id": hex::encode(&r.instance_id),
                    "result_ref": hex::encode(&r.result_ref),
                    "nd_class": r.nd_class,
                    "seq": r.seq,
                    "react_turn": r.react_turn,
                    "react_branch": r.react_branch,
                })
            })
            .collect();
        json!({ "records": records, "has_more": resp.has_more }).to_string()
    } else if resp.records.is_empty() {
        "(no capture records)".to_string()
    } else {
        let mut out = String::new();
        for r in &resp.records {
            let _ = write!(
                out,
                "{}{}  inst {}  result {}  nd {}  seq {}{}",
                if out.is_empty() { "" } else { "\n" },
                hex::encode(&r.mote_id),
                hex::encode(&r.instance_id),
                hex::encode(&r.result_ref),
                r.nd_class,
                r.seq,
                r.react_turn.map_or_else(String::new, |turn| format!(
                    "  react turn {turn} {}",
                    r.react_branch
                )),
            );
        }
        if resp.has_more {
            out.push_str("\n(more — raise --limit)");
        }
        out
    }
}

// ----- MM-3 (D110) secrets + D113 triggers -----

/// Map a [`proto::TriggerKind`] discriminant to a stable display name. An
/// out-of-range value renders `unspecified` (forward-compatible).
#[must_use]
pub fn trigger_kind_name(kind: i32) -> &'static str {
    match proto::TriggerKind::try_from(kind) {
        Ok(proto::TriggerKind::Webhook) => "webhook",
        Ok(proto::TriggerKind::Cron) => "cron",
        Ok(proto::TriggerKind::Grpc) => "grpc",
        _ => "unspecified",
    }
}

/// Map a [`proto::TriggerAuth`] discriminant to a stable display name. An
/// out-of-range value renders `unspecified` (forward-compatible).
#[must_use]
pub fn trigger_auth_name(auth: i32) -> &'static str {
    match proto::TriggerAuth::try_from(auth) {
        Ok(proto::TriggerAuth::None) => "none",
        Ok(proto::TriggerAuth::HmacSha256) => "hmac_sha256",
        Ok(proto::TriggerAuth::Bearer) => "bearer",
        _ => "unspecified",
    }
}

/// Render `secrets set` — whether the secret was stored. SN-8/D110: the VALUE is
/// write-only — it never round-trips back in any response.
#[must_use]
pub fn render_put_secret(resp: &proto::PutSecretResponse, json: bool) -> String {
    if json {
        json!({ "stored": resp.stored }).to_string()
    } else if resp.stored {
        "stored".to_string()
    } else {
        "not stored".to_string()
    }
}

/// Render `secrets list` — the stored secret NAMES + timestamps (never the value).
#[must_use]
pub fn render_secret_names(resp: &proto::ListSecretNamesResponse, json: bool) -> String {
    if json {
        let names: Vec<Value> = resp
            .names
            .iter()
            .map(|n| {
                json!({
                    "name": n.name,
                    "created_unix_ms": n.created_unix_ms,
                    "updated_unix_ms": n.updated_unix_ms,
                })
            })
            .collect();
        json!({ "names": names, "has_more": resp.has_more }).to_string()
    } else if resp.names.is_empty() {
        "(no secrets stored)".to_string()
    } else {
        resp.names
            .iter()
            .map(|n| n.name.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `secrets rm` — whether a secret was removed.
#[must_use]
pub fn render_delete_secret(resp: &proto::DeleteSecretResponse, json: bool) -> String {
    if json {
        json!({ "removed": resp.removed }).to_string()
    } else if resp.removed {
        "removed".to_string()
    } else {
        "not removed (no such secret)".to_string()
    }
}

/// Render `triggers add` — the server-derived trigger id (SN-8).
#[must_use]
pub fn render_register_trigger(resp: &proto::RegisterTriggerResponse, json: bool) -> String {
    if json {
        json!({ "trigger_id": hex::encode(&resp.trigger_id) }).to_string()
    } else {
        format!("registered trigger_id={}", hex::encode(&resp.trigger_id))
    }
}

/// Render `triggers list` — the registered triggers + their binding. A credential
/// is shown as a presence flag only (never the secret value, D81).
#[must_use]
pub fn render_triggers_list(resp: &proto::ListTriggersResponse, json: bool) -> String {
    if json {
        let triggers: Vec<Value> = resp
            .triggers
            .iter()
            .map(|t| {
                json!({
                    "trigger_id": hex::encode(&t.trigger_id),
                    "name": t.name,
                    "kind": trigger_kind_name(t.kind),
                    "recipe_handle": t.recipe_handle,
                    "auth": trigger_auth_name(t.auth),
                    "auth_secret_present": t.auth_secret_present,
                    "schedule_spec": t.schedule_spec,
                    "enabled": t.enabled,
                    "last_fire_unix_ms": t.last_fire_unix_ms,
                })
            })
            .collect();
        json!({ "triggers": triggers, "has_more": resp.has_more }).to_string()
    } else if resp.triggers.is_empty() {
        "(no triggers registered)".to_string()
    } else {
        resp.triggers
            .iter()
            .map(|t| {
                let state = if t.enabled { "enabled" } else { "disabled" };
                let secret = if t.auth_secret_present {
                    "  secret"
                } else {
                    ""
                };
                let sched = if t.schedule_spec.is_empty() {
                    String::new()
                } else {
                    format!("  schedule={}", t.schedule_spec)
                };
                format!(
                    "{}  [{}]  -> {}  auth={}  ({}){}{}",
                    t.name,
                    trigger_kind_name(t.kind),
                    t.recipe_handle,
                    trigger_auth_name(t.auth),
                    state,
                    secret,
                    sched,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render `triggers rm` — whether a trigger was removed.
#[must_use]
pub fn render_deregister_trigger(resp: &proto::DeregisterTriggerResponse, json: bool) -> String {
    if json {
        json!({ "removed": resp.removed }).to_string()
    } else if resp.removed {
        "removed".to_string()
    } else {
        "not removed (no such trigger)".to_string()
    }
}

/// Render `triggers fire` (`SubmitTrigger`) — the started run + dedup signal. A
/// deduped event returns the PRIOR run's instance id (idempotent).
#[must_use]
pub fn render_submit_trigger(resp: &proto::SubmitTriggerResponse, json: bool) -> String {
    if json {
        json!({
            "instance_id": hex::encode(&resp.instance_id),
            "deduped": resp.deduped,
        })
        .to_string()
    } else {
        format!(
            "instance_id {}  deduped={}",
            hex::encode(&resp.instance_id),
            resp.deduped
        )
    }
}

/// Render `triggers test` — the dry-run validation outcome (no run is fired).
#[must_use]
pub fn render_test_trigger(resp: &proto::TestTriggerResponse, json: bool) -> String {
    if json {
        json!({ "ok": resp.ok, "detail": resp.detail }).to_string()
    } else if resp.ok {
        format!("ok ({})", resp.detail)
    } else {
        format!("invalid ({})", resp.detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wait::{WaitOutcome, WaitState};

    #[test]
    fn critic_verdict_summary_decodes_judge_and_ignores_prose() {
        use kx_critic_types::{CriticReason, CriticVerdict};
        // A committed judge verdict decodes to a readable summary.
        assert_eq!(
            critic_verdict_summary(&CriticVerdict::Valid.encode()).as_deref(),
            Some("valid")
        );
        let invalid = CriticVerdict::Invalid {
            reason: CriticReason::JudgeRejected { reason_code: 0 },
        }
        .encode();
        assert_eq!(
            critic_verdict_summary(&invalid).as_deref(),
            Some("invalid: judge: answer did not satisfy the rubric")
        );
        // A plain model answer (not a verdict) is left alone (raw display).
        assert_eq!(critic_verdict_summary(b"Paris"), None);
        assert_eq!(critic_verdict_summary(b""), None);
    }

    #[test]
    fn render_wait_shows_decoded_verdict_for_a_judge_terminal() {
        use kx_critic_types::CriticVerdict;
        let outcome = WaitOutcome {
            instance_id: vec![1; 16],
            terminal_mote_id: vec![2; 32],
            state: WaitState::Committed,
            result_ref: Some(vec![3; 32]),
            payload: Some(CriticVerdict::Valid.encode()),
        };
        let text = render_wait(&outcome, false, true);
        assert!(text.contains("verdict          valid"), "got: {text}");
        // The non-UTF-8 bincode is NOT dumped as redundant hex when decoded.
        assert!(!text.contains("result_hex"), "got: {text}");
    }

    #[test]
    fn state_names_cover_range_and_unknown() {
        use proto::MoteSnapshotState as S;
        assert_eq!(state_name(S::Committed as i32), "COMMITTED");
        assert_eq!(state_name(S::Failed as i32), "FAILED");
        assert_eq!(state_name(S::Pending as i32), "PENDING");
        assert_eq!(state_name(999), "UNKNOWN");
    }

    #[test]
    fn invoke_json_has_hex_ids() {
        let resp = proto::InvokeResponse {
            instance_id: vec![0xab; 16],
            recipe_fingerprint: vec![0xcd; 32],
            terminal_mote_id: vec![0xef; 32],
            react_chain_salt: Vec::new(),
        };
        let v: Value = serde_json::from_str(&render_invoke(&resp, true)).unwrap();
        assert_eq!(v["instance_id"].as_str().unwrap().len(), 32); // 16B -> 32 hex
        assert_eq!(v["terminal_mote_id"].as_str().unwrap().len(), 64); // 32B -> 64 hex
        assert_eq!(v["recipe_fingerprint"], json!("cd".repeat(32)));
    }

    #[test]
    fn projection_json_renders_states_and_refs() {
        let view = proto::ProjectionView {
            instance_id: vec![1u8; 16],
            recipe_fingerprint: vec![2u8; 32],
            current_seq: 7,
            motes: vec![proto::MoteSnapshot {
                mote_id: vec![3u8; 32],
                state: proto::MoteSnapshotState::Committed as i32,
                nd_class: 1,
                promotion: 1,
                result_ref: Some(vec![4u8; 32]),
                warrant_ref: None,
                mote_def_hash: vec![5u8; 32],
                committed_seq: Some(7),
                parents: vec![proto::ParentRef {
                    parent_id: vec![9u8; 32],
                    edge_kind: proto::EdgeKind::Data as i32,
                    non_cascade: false,
                }],
                verdict: None,
                anomaly: None,
            }],
        };
        let v: Value = serde_json::from_str(&render_projection(&view, true)).unwrap();
        assert_eq!(v["current_seq"], 7);
        assert_eq!(v["motes"][0]["state"], "COMMITTED");
        assert_eq!(v["motes"][0]["result_ref"].as_str().unwrap().len(), 64);
        // The DAG edge surfaces in --json (T-XSURF-1): parent_id hex + raw
        // edge_kind discriminant + non_cascade (parity with the Python MoteView).
        assert_eq!(
            v["motes"][0]["parents"][0]["parent_id"],
            hex::encode(&[9u8; 32])
        );
        assert_eq!(v["motes"][0]["parents"][0]["edge_kind"], "data"); // EDGE_KIND_DATA → name
        assert_eq!(v["motes"][0]["parents"][0]["non_cascade"], false);
        // Human form mentions the state name + the seq + the edge.
        let human = render_projection(&view, false);
        assert!(human.contains("COMMITTED") && human.contains("seq 7"));
        assert!(human.contains("parents=") && human.contains(":data"));
    }

    #[test]
    fn wait_committed_json_has_utf8_and_hex() {
        let outcome = WaitOutcome {
            instance_id: vec![1u8; 16],
            terminal_mote_id: vec![2u8; 32],
            state: WaitState::Committed,
            result_ref: Some(vec![3u8; 32]),
            payload: Some(b"hello".to_vec()),
        };
        let v: Value = serde_json::from_str(&render_wait(&outcome, true, true)).unwrap();
        assert_eq!(v["state"], "COMMITTED");
        assert_eq!(v["result_utf8"], "hello");
        assert_eq!(v["result_len"], 5);
        assert_eq!(v["result_hex"], hex::encode(b"hello"));
        // With include_payload=false (–-out path), the bytes are omitted.
        let meta: Value = serde_json::from_str(&render_wait(&outcome, true, false)).unwrap();
        assert!(meta.get("result_hex").is_none());
        assert_eq!(meta["result_len"], 5);
    }

    #[test]
    fn wait_running_json_flags_timeout() {
        let outcome = WaitOutcome {
            instance_id: vec![1u8; 16],
            terminal_mote_id: vec![2u8; 32],
            state: WaitState::Running,
            result_ref: None,
            payload: None,
        };
        let v: Value = serde_json::from_str(&render_wait(&outcome, true, true)).unwrap();
        assert_eq!(v["state"], "RUNNING");
        assert_eq!(v["timed_out"], true);
    }

    #[test]
    fn agent_result_failed_with_tool_actions_names_the_tool_loop() {
        // W2: a dead-lettered agent that FIRED tools but never settled reports the
        // tool-loop specifically (the honest terminal, not a generic failure).
        let outcome = WaitOutcome {
            instance_id: vec![1u8; 16],
            terminal_mote_id: vec![2u8; 32],
            state: WaitState::Failed,
            result_ref: None,
            payload: None,
        };
        let actions = vec![("mcp-echo/echo".to_string(), "1".to_string(), 0u32, 0u32)];
        let human = render_agent_result(&outcome, &actions, false);
        assert!(
            human.contains("exhausted its tool-call budget without settling"),
            "names the W2 tool-loop: {human}"
        );
        // No tool actions ⇒ the generic failure message (e.g. all proposals refused).
        let generic = render_agent_result(&outcome, &[], false);
        assert!(generic.contains("the agent run failed"));
        assert!(!generic.contains("tool-call budget"));
    }

    #[test]
    fn global_delta_json_uses_type_tag_instance_hex_and_nd_strings() {
        use proto::global_event_delta::Kind;
        // committed: attributed, nd_class as the lowercase wire tag.
        let committed = proto::GlobalEventDelta {
            seq: 9,
            instance_id: vec![0x5a; 16],
            kind: Some(Kind::Committed(proto::CommittedDelta {
                mote_id: vec![7u8; 32],
                result_ref: vec![8u8; 32],
                nd_class: proto::NdClass::Pure as i32,
            })),
        };
        let v: Value = serde_json::from_str(&render_global_delta(&committed, true)).unwrap();
        assert_eq!(v["type"], "committed");
        assert_eq!(v["seq"], 9);
        assert_eq!(v["instance_id"], "5a".repeat(16));
        assert_eq!(v["nd_class"], "pure");
        assert_eq!(v["mote_id"].as_str().unwrap().len(), 64);
        // run_registered: the kind the per-run wire never carries.
        let registered = proto::GlobalEventDelta {
            seq: 3,
            instance_id: vec![0x5a; 16],
            kind: Some(Kind::RunRegistered(proto::RunRegisteredDelta {
                recipe_fingerprint: vec![0xcd; 32],
                registered_unix_ms: 1_700_000_000_000,
            })),
        };
        let v: Value = serde_json::from_str(&render_global_delta(&registered, true)).unwrap();
        assert_eq!(v["type"], "run_registered");
        assert_eq!(v["recipe_fingerprint"], "cd".repeat(32));
        assert_eq!(v["registered_unix_ms"], 1_700_000_000_000u64);
        // The human form narrates a run start and shows the attribution.
        let human = render_global_delta(&registered, false);
        assert!(human.contains("RUN_STARTED") && human.contains(&"5a".repeat(16)));
        // pre-registration: instance_id is the honest empty string (JSON) / `-` (human).
        let unknown = proto::GlobalEventDelta {
            seq: 1,
            instance_id: Vec::new(),
            kind: None,
        };
        let v: Value = serde_json::from_str(&render_global_delta(&unknown, true)).unwrap();
        assert_eq!(v["type"], "unknown");
        assert_eq!(v["instance_id"], "");
        assert!(render_global_delta(&unknown, false).contains("inst=-"));
    }

    #[test]
    fn telemetry_json_parity_null_tokens_and_zero_instance() {
        let resp = proto::ListMoteTelemetryResponse {
            rows: vec![
                proto::MoteTelemetryRow {
                    mote_id: vec![1u8; 32],
                    instance_id: vec![0x5a; 16],
                    wall_clock_ms: 42,
                    input_tokens: None,
                    output_tokens: Some(17),
                    model_id: "qwen3".into(),
                    tool_id: String::new(),
                    started_unix_ms: 1_700_000_000_000,
                    seq: 11,
                },
                proto::MoteTelemetryRow {
                    mote_id: vec![2u8; 32],
                    instance_id: vec![0u8; 16], // all-zero = unattributed
                    wall_clock_ms: 5,
                    input_tokens: None,
                    output_tokens: None,
                    model_id: String::new(),
                    tool_id: "mcp-echo".into(),
                    started_unix_ms: 1_700_000_000_001,
                    seq: 7,
                },
            ],
            has_more: true,
        };
        let v: Value = serde_json::from_str(&render_telemetry(&resp, true)).unwrap();
        assert_eq!(v["rows"][0]["instance_id"], "5a".repeat(16));
        assert!(v["rows"][0]["input_tokens"].is_null());
        assert_eq!(v["rows"][0]["output_tokens"], 17);
        assert_eq!(v["rows"][0]["wall_clock_ms"], 42);
        assert_eq!(v["rows"][0]["model_id"], "qwen3");
        assert_eq!(v["rows"][0]["started_unix_ms"], 1_700_000_000_000u64);
        // The all-zero attribution renders as the honest empty string.
        assert_eq!(v["rows"][1]["instance_id"], "");
        assert_eq!(v["rows"][1]["tool_id"], "mcp-echo");
        assert_eq!(v["has_more"], true);
        // Human form: the cursor hint names the last row's seq.
        let human = render_telemetry(&resp, false);
        assert!(human.contains("--before-seq 7"));
        assert!(
            human.contains("inst -"),
            "all-zero attribution shows a dash"
        );
        // Empty: an honest placeholder, not an empty string.
        let empty = proto::ListMoteTelemetryResponse {
            rows: vec![],
            has_more: false,
        };
        assert_eq!(render_telemetry(&empty, false), "(no telemetry rows)");
    }

    #[test]
    #[allow(clippy::too_many_lines)] // exhaustively mirrors 3 RPCs' wire field names
    fn replan_react_capture_json_mirror_proto_field_names() {
        let replan = proto::ListReplanRoundsResponse {
            rounds: vec![proto::ReplanRoundSummary {
                round: 1,
                shaper_mote_id: vec![3u8; 32],
                model_id: "qwen3".into(),
                failed_step_ids: vec![vec![4u8; 32]],
                escalated: false,
                seq: 21,
            }],
            has_more: false,
        };
        let v: Value = serde_json::from_str(&render_replan_rounds(&replan, true)).unwrap();
        assert_eq!(v["rounds"][0]["round"], 1);
        assert_eq!(v["rounds"][0]["shaper_mote_id"], "03".repeat(32));
        assert_eq!(v["rounds"][0]["failed_step_ids"][0], "04".repeat(32));
        assert_eq!(v["rounds"][0]["escalated"], false);
        assert_eq!(v["has_more"], false);

        let react = proto::ListReactTurnsResponse {
            turns: vec![
                proto::ReactTurnSummary {
                    turn: 2,
                    turn_mote_id: vec![5u8; 32],
                    instance_id: vec![6u8; 16],
                    model_id: "qwen3".into(),
                    branch: "tool".into(),
                    tool_id: "mcp-echo".into(),
                    tool_version: "1".into(),
                    max_turns: 8,
                    max_tool_calls: 6,
                    seq: 33,
                    rejection_reason: String::new(),
                    step_salt: Vec::new(),
                    call_index: 0,
                },
                // PR-3 (A2): a rejected turn carries its reason on both surfaces.
                proto::ReactTurnSummary {
                    turn: 1,
                    turn_mote_id: vec![5u8; 32],
                    instance_id: vec![6u8; 16],
                    model_id: "qwen3".into(),
                    branch: "rejected".into(),
                    tool_id: String::new(),
                    tool_version: String::new(),
                    max_turns: 8,
                    max_tool_calls: 6,
                    seq: 32,
                    rejection_reason: "args do not match inputSchema".into(),
                    step_salt: Vec::new(),
                    call_index: 0,
                },
            ],
            has_more: true,
        };
        let v: Value = serde_json::from_str(&render_react_turns(&react, true)).unwrap();
        assert_eq!(v["turns"][0]["turn_mote_id"], "05".repeat(32));
        // The rejected turn surfaces its reason in JSON + human output.
        assert_eq!(v["turns"][1]["branch"], "rejected");
        assert_eq!(
            v["turns"][1]["rejection_reason"],
            "args do not match inputSchema"
        );
        assert_eq!(v["turns"][0]["instance_id"], "06".repeat(16));
        assert_eq!(v["turns"][0]["branch"], "tool");
        assert_eq!(v["turns"][0]["max_tool_calls"], 6);
        assert_eq!(v["has_more"], true);
        let human = render_react_turns(&react, false);
        // T-MULTI-ELEMENT-TOOLCALLS: a tool turn shows its call_index (`tool[0]`).
        assert!(human.contains("tool[0] mcp-echo@1") && human.contains("caps 8/6"));
        assert!(
            human.contains("branch rejected") && human.contains("reason args do not match"),
            "the rejected turn shows its reason in human output: {human}"
        );

        let capture = proto::ListCaptureRecordsResponse {
            records: vec![proto::CaptureRecordSummary {
                mote_id: vec![7u8; 32],
                instance_id: vec![8u8; 16],
                result_ref: vec![9u8; 32],
                nd_class: "pure".into(),
                seq: 44,
                react_turn: None,
                react_branch: String::new(),
            }],
            has_more: false,
        };
        let v: Value = serde_json::from_str(&render_capture_records(&capture, true)).unwrap();
        assert_eq!(v["records"][0]["mote_id"], "07".repeat(32));
        assert_eq!(v["records"][0]["nd_class"], "pure");
        assert!(v["records"][0]["react_turn"].is_null());
        assert_eq!(v["records"][0]["react_branch"], "");
        // Empty placeholders are honest.
        assert_eq!(
            render_replan_rounds(
                &proto::ListReplanRoundsResponse {
                    rounds: vec![],
                    has_more: false
                },
                false
            ),
            "(no replan rounds)"
        );
        assert_eq!(
            render_react_turns(
                &proto::ListReactTurnsResponse {
                    turns: vec![],
                    has_more: false
                },
                false
            ),
            "(no react turns)"
        );
        assert_eq!(
            render_capture_records(
                &proto::ListCaptureRecordsResponse {
                    records: vec![],
                    has_more: false
                },
                false
            ),
            "(no capture records)"
        );
    }

    #[test]
    fn delta_committed_renders_both_forms() {
        let delta = proto::EventDelta {
            seq: 5,
            kind: Some(proto::event_delta::Kind::Committed(proto::CommittedDelta {
                mote_id: vec![7u8; 32],
                result_ref: vec![8u8; 32],
                nd_class: 1,
            })),
        };
        let human = render_delta(&delta, false).unwrap();
        assert!(human.contains("seq 5 COMMITTED"));
        let v: Value = serde_json::from_str(&render_delta(&delta, true).unwrap()).unwrap();
        assert_eq!(v["kind"], "committed");
        assert_eq!(v["seq"], 5);
        // A delta with no kind is skipped.
        assert!(render_delta(&proto::EventDelta { seq: 1, kind: None }, false).is_none());
    }

    #[test]
    fn alerts_json_carries_every_snake_case_field() {
        // Locks the tri-surface parity contract: the CLI `--json` shape MUST carry
        // every field the Py/TS SDKs expose (GR16 — a dropped `reason_code` was the
        // exact drift this guards). If a proto field is added, extend this set.
        let resp = proto::ListAlertsResponse {
            alerts: vec![proto::AlertSummary {
                alert_id: vec![0x11; 16],
                mote_id: vec![0x22; 32],
                instance_id: vec![0x33; 16],
                reason_class: "dead_lettered".into(),
                severity: "error".into(),
                seq: 7,
                created_unix_ms: 123,
                reason_code: 8,
            }],
            has_more: true,
        };
        let v: Value = serde_json::from_str(&render_alerts(&resp, true)).unwrap();
        let row = &v["alerts"][0];
        for key in [
            "alert_id",
            "mote_id",
            "instance_id",
            "reason_class",
            "reason_code",
            "severity",
            "seq",
            "created_unix_ms",
        ] {
            assert!(!row[key].is_null(), "alerts --json must carry `{key}`");
        }
        assert_eq!(
            row["reason_code"], 8,
            "the numeric discriminant for label reuse"
        );
        assert_eq!(row["reason_class"], "dead_lettered");
        assert_eq!(v["has_more"], true);
        // The empty case is the honest healthy state, never a fabricated row.
        let empty = render_alerts(
            &proto::ListAlertsResponse {
                alerts: vec![],
                has_more: false,
            },
            false,
        );
        assert!(empty.contains("System is healthy"));
    }

    #[test]
    fn telemetry_summary_json_carries_every_snake_case_field() {
        // Locks the tri-surface parity contract for W1a-3: the CLI `--json` shape
        // MUST carry every field the Py/TS SDKs expose. If a proto field is added,
        // extend this set.
        let resp = proto::ListTelemetrySummaryResponse {
            rows: vec![proto::ModelTokenRollup {
                model_id: "kx-serve:qwen3".into(),
                count: 3,
                total_output_tokens: 60,
                total_wall_clock_ms: 12,
            }],
            total_motes: 4,
            total_output_tokens: 60,
        };
        let v: Value = serde_json::from_str(&render_telemetry_summary(&resp, true)).unwrap();
        let row = &v["rows"][0];
        for key in [
            "model_id",
            "count",
            "total_output_tokens",
            "total_wall_clock_ms",
        ] {
            assert!(!row[key].is_null(), "summary --json must carry `{key}`");
        }
        assert_eq!(row["model_id"], "kx-serve:qwen3");
        assert_eq!(row["count"], 3);
        assert_eq!(v["total_motes"], 4);
        assert_eq!(v["total_output_tokens"], 60);
        // The empty case is honest, never a fabricated row.
        let empty = render_telemetry_summary(
            &proto::ListTelemetrySummaryResponse {
                rows: vec![],
                total_motes: 0,
                total_output_tokens: 0,
            },
            false,
        );
        assert!(empty.contains("no telemetry rows"));
    }
}
