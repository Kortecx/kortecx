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
            let _ = write!(
                out,
                "\n  {}  {:<12} nd={} result={} committed_seq={}",
                hex::encode(&m.mote_id),
                state_name(m.state),
                m.nd_class,
                hex::encode_opt(m.result_ref.as_deref()),
                m.committed_seq
                    .map_or_else(|| "-".to_string(), |s| s.to_string()),
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
            if include_payload {
                match std::str::from_utf8(payload) {
                    Ok(text) => {
                        let _ = write!(out, "\nresult           {text}");
                    }
                    Err(_) => {
                        let _ = write!(out, "\nresult_hex       {}", hex::encode(payload));
                    }
                }
            }
        }
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
                })
            })
            .collect();
        json!({ "models": models }).to_string()
    } else if resp.models.is_empty() {
        "(no models on this serve)".to_string()
    } else {
        resp.models
            .iter()
            .map(|m| {
                format!(
                    "{}  [{}]  ctx={}  {}{}",
                    m.model_id,
                    m.modalities.join("+"),
                    m.context_len,
                    m.description,
                    if m.serving { "  (serving)" } else { "" }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wait::{WaitOutcome, WaitState};

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
                parents: vec![],
                verdict: None,
                anomaly: None,
            }],
        };
        let v: Value = serde_json::from_str(&render_projection(&view, true)).unwrap();
        assert_eq!(v["current_seq"], 7);
        assert_eq!(v["motes"][0]["state"], "COMMITTED");
        assert_eq!(v["motes"][0]["result_ref"].as_str().unwrap().len(), 64);
        // Human form mentions the state name + the seq.
        let human = render_projection(&view, false);
        assert!(human.contains("COMMITTED") && human.contains("seq 7"));
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
}
