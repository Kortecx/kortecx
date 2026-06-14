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
                })
            })
            .collect();
        json!({ "turns": turns, "has_more": resp.has_more }).to_string()
    } else if resp.turns.is_empty() {
        "(no react turns)".to_string()
    } else {
        let mut out = String::new();
        for t in &resp.turns {
            let _ = write!(
                out,
                "{}turn {}  inst {}  branch {}{}  model {}  caps {}/{}  seq {}",
                if out.is_empty() { "" } else { "\n" },
                t.turn,
                hex::encode(&t.instance_id),
                t.branch,
                if t.tool_id.is_empty() {
                    String::new()
                } else {
                    format!(" tool {}@{}", t.tool_id, t.tool_version)
                },
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
            turns: vec![proto::ReactTurnSummary {
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
            }],
            has_more: true,
        };
        let v: Value = serde_json::from_str(&render_react_turns(&react, true)).unwrap();
        assert_eq!(v["turns"][0]["turn_mote_id"], "05".repeat(32));
        assert_eq!(v["turns"][0]["instance_id"], "06".repeat(16));
        assert_eq!(v["turns"][0]["branch"], "tool");
        assert_eq!(v["turns"][0]["max_tool_calls"], 6);
        assert_eq!(v["has_more"], true);
        let human = render_react_turns(&react, false);
        assert!(human.contains("tool mcp-echo@1") && human.contains("caps 8/6"));

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
}
