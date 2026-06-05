//! R4 — the off-truth-path audit trail, end-to-end over the PUBLIC engine path
//! (`kx_runtime::run` + `RuntimeConfig.audit_log`, i.e. exactly what
//! `kx run --audit-log <path>` drives). These tests prove the headline guarantees:
//! auditing never perturbs the truth (digest `a6b5c679…` invariant with audit ON),
//! the trail is a complete + ordered + deterministic JSONL stream, and recovery is
//! audited as a re-read (zero re-dispatch) that still covers every committed Mote.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use kx_runtime::config::Mode;
use kx_runtime::RuntimeConfig;
use serde_json::Value;

/// The canonical projection digest (schema v5) — the durability law: audit ON,
/// audit OFF, or inspected, the committed-facts digest is byte-identical.
const CANONICAL_DIGEST: &str = "a6b5c67939f14bfcbd125f7461b2bd0e481f6ee2fc98c1ab638730e2d2ace2e9";

fn read_jsonl(path: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).expect("audit log written");
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("each audit line is valid JSON"))
        .collect()
}

fn types(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            l["type"]
                .as_str()
                .expect("every line is tagged")
                .to_string()
        })
        .collect()
}

fn count(lines: &[Value], ty: &str) -> usize {
    types(lines).iter().filter(|t| *t == ty).count()
}

fn is_hex64(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

fn run_cfg(dir: &Path, mode: Mode, audit: &Path) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("journal.sqlite"),
        content_root: dir.join("content"),
        mode,
        crash_at: None,
        checkpoint_every: Some(2),
        audit_log: Some(audit.to_path_buf()),
    }
}

#[test]
fn audit_off_preserves_canonical_digest() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RuntimeConfig {
        journal_path: dir.path().join("j.sqlite"),
        content_root: dir.path().join("c"),
        mode: Mode::Run,
        crash_at: None,
        checkpoint_every: Some(2),
        audit_log: None,
    };
    let out = kx_runtime::run(&cfg).unwrap();
    assert_eq!((out.committed, out.total), (8, 8));
    assert_eq!(out.digest.to_hex(), CANONICAL_DIGEST);
}

#[test]
fn audit_on_preserves_digest_and_writes_ordered_trail() {
    let dir = tempfile::tempdir().unwrap();
    let audit = dir.path().join("audit.jsonl");
    let out = kx_runtime::run(&run_cfg(dir.path(), Mode::Run, &audit)).unwrap();

    // The durability law: audit ON does not change the committed-facts digest.
    assert_eq!((out.committed, out.total), (8, 8));
    assert_eq!(
        out.digest.to_hex(),
        CANONICAL_DIGEST,
        "audit must not perturb the digest"
    );

    let lines = read_jsonl(&audit);
    let tys = types(&lines);

    // Envelope: seq is gap-free from 0, ts_ms present + numeric on every line.
    for (i, l) in lines.iter().enumerate() {
        assert_eq!(l["seq"].as_u64(), Some(i as u64), "seq is gap-free from 0");
        assert!(
            l["ts_ms"].is_u64(),
            "ts_ms present + numeric (off the digest)"
        );
        assert!(l.get("principal").is_none(), "no principal unless set");
    }

    // Shape: starts with run_started, ends with run_completed, one children_derived.
    assert_eq!(tys.first().map(String::as_str), Some("run_started"));
    assert_eq!(tys.last().map(String::as_str), Some("run_completed"));
    assert_eq!(
        count(&lines, "children_derived"),
        1,
        "the one shaper unrolls once"
    );

    // The canonical demo dispatches 8 Motes and commits 8.
    assert_eq!(
        count(&lines, "mote_dispatched"),
        8,
        "8 dispatches in the clean run"
    );
    assert_eq!(
        count(&lines, "mote_committed"),
        8,
        "8 committed Motes in the sweep"
    );

    // The terminal sweep runs AFTER the loop: every mote_committed comes after every
    // mote_dispatched.
    let last_dispatch = tys.iter().rposition(|t| t == "mote_dispatched").unwrap();
    let first_commit = tys.iter().position(|t| t == "mote_committed").unwrap();
    assert!(
        last_dispatch < first_commit,
        "committed sweep is terminal (after all dispatch)"
    );

    // children_derived happens during the loop (before the terminal sweep).
    let children = tys.iter().position(|t| t == "children_derived").unwrap();
    assert!(children < first_commit, "children derive during the loop");

    // The committed set == the dispatched set (8 unique hex ids), and each is hex.
    let dispatched: std::collections::BTreeSet<String> = lines
        .iter()
        .filter(|l| l["type"] == "mote_dispatched")
        .map(|l| {
            assert!(
                is_hex64(&l["mote_id"]),
                "dispatched mote_id is 64-hex, not an int array"
            );
            l["mote_id"].as_str().unwrap().to_string()
        })
        .collect();
    let committed: std::collections::BTreeSet<String> = lines
        .iter()
        .filter(|l| l["type"] == "mote_committed")
        .map(|l| {
            assert!(is_hex64(&l["mote_id"]), "committed mote_id is 64-hex");
            assert!(is_hex64(&l["result_ref"]), "result_ref is 64-hex");
            let nd = l["nd_class"].as_str().unwrap();
            assert!(
                matches!(nd, "pure" | "read_only_nondet" | "world_mutating"),
                "nd_class is a known class (no float/similarity on the audit path), got {nd:?}"
            );
            l["mote_id"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(dispatched.len(), 8);
    assert_eq!(
        dispatched, committed,
        "the audited committed set == the dispatched set"
    );

    // run_completed is the tamper-evident receipt: 8/8 + the canonical digest hex.
    let done = lines.last().unwrap();
    assert_eq!(done["committed"].as_u64(), Some(8));
    assert_eq!(done["total"].as_u64(), Some(8));
    assert_eq!(
        done["digest"].as_str(),
        Some(CANONICAL_DIGEST),
        "RunCompleted carries the product digest"
    );

    // Every dispatch carries a known kind (the mapping is exercised per-variant in
    // the engine unit test `action_accessors_echo_the_picked_mote`).
    for l in lines.iter().filter(|l| l["type"] == "mote_dispatched") {
        let kind = l["kind"].as_str().unwrap();
        assert!(
            matches!(kind, "pure" | "critic" | "wm_fresh" | "wm_recovery"),
            "dispatch kind is one of the known dispatch paths, got {kind:?}"
        );
    }
}

#[test]
fn audit_trail_is_deterministic_across_runs() {
    // Two independent runs produce byte-identical trails modulo the wall-clock
    // (ts_ms) — the typed events carry no time, so order + ids + counts are stable.
    let normalize = |lines: Vec<Value>| -> Vec<Value> {
        lines
            .into_iter()
            .map(|mut l| {
                if let Some(obj) = l.as_object_mut() {
                    obj.remove("ts_ms");
                }
                l
            })
            .collect::<Vec<_>>()
    };

    let dir_a = tempfile::tempdir().unwrap();
    let audit_a = dir_a.path().join("a.jsonl");
    kx_runtime::run(&run_cfg(dir_a.path(), Mode::Run, &audit_a)).unwrap();

    let dir_b = tempfile::tempdir().unwrap();
    let audit_b = dir_b.path().join("b.jsonl");
    kx_runtime::run(&run_cfg(dir_b.path(), Mode::Run, &audit_b)).unwrap();

    assert_eq!(
        normalize(read_jsonl(&audit_a)),
        normalize(read_jsonl(&audit_b)),
        "the audit trail is deterministic (time excluded)"
    );
}

#[test]
fn recovery_is_audited_as_reread_not_rerun() {
    // Run once to a complete journal, then REPLAY the same journal with audit on.
    // The headline novel claim, made observable: recovery RE-READS the committed
    // facts (zero re-dispatch) yet the audit trail still covers every committed
    // Mote (the terminal sweep reads the folded projection, not the loop).
    let dir = tempfile::tempdir().unwrap();
    let audit1 = dir.path().join("run.jsonl");
    let first = kx_runtime::run(&run_cfg(dir.path(), Mode::Run, &audit1)).unwrap();
    assert_eq!((first.committed, first.total), (8, 8));

    let audit2 = dir.path().join("replay.jsonl");
    let replay = kx_runtime::run(&run_cfg(dir.path(), Mode::Replay, &audit2)).unwrap();
    assert_eq!((replay.committed, replay.total), (8, 8));
    assert_eq!(
        replay.digest.to_hex(),
        CANONICAL_DIGEST,
        "replay digest == original"
    );

    let lines = read_jsonl(&audit2);

    // Exactly one Recovered, reporting the folded frontier (all 8 already committed).
    assert_eq!(
        count(&lines, "recovered"),
        1,
        "replay folds an existing journal once"
    );
    let rec = lines.iter().find(|l| l["type"] == "recovered").unwrap();
    assert_eq!(
        rec["committed_through"].as_u64(),
        Some(8),
        "8 committed Motes recovered"
    );
    assert!(
        rec["folded_through"].as_u64().unwrap() > 0,
        "folded past the empty journal"
    );

    // The novel claim, audited: NOTHING is re-dispatched on a fully-committed replay.
    assert_eq!(
        count(&lines, "mote_dispatched"),
        0,
        "recovery re-reads, never re-runs"
    );
    // …yet the trail still covers every committed Mote (the recovery-safe sweep).
    assert_eq!(
        count(&lines, "mote_committed"),
        8,
        "all committed Motes audited on replay"
    );
    assert_eq!(
        types(&lines).last().map(String::as_str),
        Some("run_completed"),
        "replay completes the trail"
    );
}
