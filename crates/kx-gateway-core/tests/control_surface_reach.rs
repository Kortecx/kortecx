// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! An authoring capability you cannot drive from the CLI is half-shipped.
//!
//! ## The incident
//!
//! The durable Workflow entity landed with `SaveWorkflow` / `RunWorkflow` /
//! `DeleteWorkflow` on the wire, in both SDKs, and in the console — and with no
//! `kx workflow` verb at all. Nothing in the build knew the CLI was a surface
//! those RPCs owed anything to, so the gap was invisible until someone went
//! looking. The same had already happened to the script registry.
//!
//! ## Why this guard is deliberately narrow
//!
//! The tempting rule is "every RPC reaches every facade". When this guard was
//! written that rule started life failing on 36 entries (CLI 93/115, Python
//! 104/115, TypeScript 112/115) and would immediately have needed a 36-line
//! exemption list. A guard whose exemption list is larger than its enforcement is
//! a rubber stamp.
//!
//! **Those numbers are historical — re-measured 2026-08-01 the surface is 121
//! RPCs and reachability is now near-total**, so the sentence above no longer
//! describes the tree and must not be read as a current measurement:
//!
//! ```text
//! CLI          118/121   missing SubmitRun (declared below),
//!                        ProposeControlAction + DescribeControlSurface (no
//!                        `kx control` verb, by design)
//! Python       121/121   the flat `_stub.<Rpc>(` probe finds 118; the three
//!                        server streams are wired in events.py as `stub.<Rpc>(`
//! TypeScript   121/121   the flat `this.grpc.<rpc>(` probe finds 118; the three
//!                        streams use a functional form, `streamEvents(this.grpc,…)`
//! ```
//!
//! ⚠ **Reachability is not an oracle.** These counts say a verb is *callable* from
//! a facade, not that it *works*. The model-driven bench drives only **15 of 121**
//! RPCs (12.4%), and the console has never been exercised against a real model at
//! all (0/121). Do not let a green count here read as coverage.
//!
//! So the rule is scoped to what actually matters: **a MUTATION in an authoring
//! domain must be reachable from the CLI.** Reads are exempt — `ProposeWorkflow`
//! writes nothing and owes no operator verb. That yields a handful of entries a
//! reviewer can hold in their head.
//!
//! ## What it found when it was first run
//!
//! Against the tree before the verbs existed, with an empty exemption list, this
//! guard failed naming exactly five RPCs:
//!
//! ```text
//! authoring mutations with no CLI verb and no declared reason:
//!     ["SaveWorkflow", "RunWorkflow", "DeleteWorkflow",
//!      "RegisterScript", "DeregisterScript"]
//! ```
//!
//! That is the replay. A guard that goes green against the incident that
//! motivated it is decoration; this one named it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_gateway_core::control_surface::{facet, is_authoring, Effect};
use kx_proto::control::GatewayRpc;
use std::path::{Path, PathBuf};

/// Authoring mutations with no CLI verb, each with the reason it is absent.
///
/// Every entry is a decision someone had to write down. Removing the last one is
/// the goal; ADDING one is the edit that has to be argued for in review.
///
/// This is not a general escape hatch — `the_exemption_list_has_no_dead_entries`
/// fails the moment a listed RPC becomes reachable, so an entry cannot outlive
/// the gap it describes.
const NOT_ON_THE_CLI: &[(&str, &str)] = &[(
    "SubmitRun",
    "BLOCKER #5: it takes a client warrant verbatim and is refused any tool \
     authority. The CLI's DAG verbs use SubmitWorkflow instead, and cli.rs says \
     so at the `invoke` help arm. This absence is the feature.",
)];

fn cli_verbs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ resolves")
        .join("kx-cli/src")
}

/// `SaveWorkflow` -> `save_workflow`, the shape a tonic client call takes.
fn snake(rpc: &str) -> String {
    let mut out = String::with_capacity(rpc.len() + 4);
    for (i, c) in rpc.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Every `.rs` under `crates/kx-cli/src`, concatenated.
fn cli_source() -> String {
    let mut out = String::new();
    let mut stack = vec![cli_verbs_dir()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
                out.push('\n');
            }
        }
    }
    assert!(
        out.len() > 10_000,
        "did not find the kx-cli sources at {} — the guard must not pass by \
         reading nothing",
        cli_verbs_dir().display()
    );
    out
}

fn is_reachable(src: &str, rpc: &str) -> bool {
    src.contains(&format!(".{}(", snake(rpc)))
}

/// Every authoring-domain MUTATION is drivable from the CLI.
#[test]
fn every_authoring_mutation_is_reachable_from_the_cli() {
    let src = cli_source();

    let missing: Vec<&str> = GatewayRpc::ALL
        .iter()
        .filter(|r| {
            let f = facet(**r);
            is_authoring(f.domain) && f.effect == Effect::Mutate
        })
        .map(|r| r.as_str())
        .filter(|name| !is_reachable(&src, name))
        .filter(|name| !NOT_ON_THE_CLI.iter().any(|(n, _)| n == name))
        .collect();

    assert!(
        missing.is_empty(),
        "authoring mutations with no CLI verb and no declared reason: {missing:?}\n\
         An authoring capability reachable from the console and the SDKs but not \
         the CLI is half-shipped: an adopter cannot script what they can author. \
         Add the verb, or add the RPC to NOT_ON_THE_CLI with the reason."
    );
}

/// A listed RPC that IS reachable must be removed from the list.
///
/// Without this the exemption list becomes a blanket: entries accumulate, nobody
/// re-checks them, and the guard quietly stops enforcing anything.
#[test]
fn the_exemption_list_has_no_dead_entries() {
    let src = cli_source();
    let dead: Vec<&str> = NOT_ON_THE_CLI
        .iter()
        .filter(|(name, _)| is_reachable(&src, name))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        dead.is_empty(),
        "these RPCs are reachable from the CLI and must be removed from \
         NOT_ON_THE_CLI: {dead:?}"
    );

    for (name, why) in NOT_ON_THE_CLI {
        assert!(
            GatewayRpc::ALL.iter().any(|r| r.as_str() == *name),
            "{name:?} is exempted but is not an RPC — a stale entry"
        );
        assert!(why.len() > 40, "{name:?} needs a real reason, got {why:?}");
    }
}

/// The snake-case mapping is right, and the reachability probe is not vacuous.
///
/// A probe that matches nothing would make the whole guard pass trivially — the
/// [[evidence-that-cannot-fail]] shape. So: pin the transform, and pin that the
/// probe finds RPCs known to be wired and misses one known not to be.
#[test]
fn the_reachability_probe_actually_discriminates() {
    assert_eq!(snake("SaveWorkflow"), "save_workflow");
    assert_eq!(snake("RegisterMcpServer"), "register_mcp_server");
    assert_eq!(snake("PutSecret"), "put_secret");

    let src = cli_source();
    // Known-wired: the CLI has had these verbs for many releases.
    for wired in ["RegisterTool", "PutSecret", "RegisterTrigger"] {
        assert!(
            is_reachable(&src, wired),
            "{wired} is wired in the CLI but the probe did not find it — the \
             probe is broken, and a broken probe passes this whole suite"
        );
    }
    // Known-absent: SubmitRun is deliberately never called by the CLI.
    assert!(
        !is_reachable(&src, "SubmitRun"),
        "SubmitRun must NOT be reachable from the CLI (BLOCKER #5)"
    );
}
