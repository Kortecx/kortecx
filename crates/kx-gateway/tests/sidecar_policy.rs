// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Every SQLite sidecar routes its open through the ONE upgrade policy.
//!
//! ## Why this is a source scan and not a behavioural test
//!
//! The policy in `crate::sidecar` decides what a schema bump does to a store: rename it
//! aside and re-import (authored work) or rebuild it empty (a derived cache). A store
//! that hand-rolls its own `DROP TABLE ... schema_version` open does not disobey that
//! policy — it never reaches it, and no behavioural test of the OTHER stores can see the
//! omission. The failure is an absence, so the check has to be over the source.
//!
//! ## Why a diff-scoped CI check was not enough
//!
//! The obvious guard — "a PR that bumps a version constant must also touch a migration
//! file" — was written first and then replayed against the incident that motivated it:
//! the journal v16→v17 bump that orphaned the v16 ladder arm. It went GREEN. That PR
//! bumped the constant, touched the migration file, and updated the CHANGELOG, and the
//! ladder arm was still missing. A guard that passes the case it exists to catch is
//! decoration. What actually caught it was a CONSTANTS-DRIVEN test asserting every
//! admitted version has an arm; this is the sidecar analogue of that test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The module that OWNS the policy, and is therefore the one place allowed to drop.
const POLICY_MODULE: &str = "sidecar.rs";

fn gateway_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![gateway_src()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn file_name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

/// Strip `#[cfg(test)] mod tests { ... }` to the end of file.
///
/// Tests legitimately construct throwaway schemas and drop tables; holding them to the
/// production policy would only teach the next author to move code into a test module.
fn production_source(path: &Path) -> String {
    let src = std::fs::read_to_string(path).unwrap();
    match src.find("\nmod tests {") {
        Some(i) => src[..i].to_string(),
        None => src,
    }
}

/// No store may drop its own tables. The policy module is the sole exception.
#[test]
fn no_sidecar_drops_tables_outside_the_policy_module() {
    let mut offenders: Vec<String> = Vec::new();
    for path in rust_sources() {
        let name = file_name(&path);
        if name == POLICY_MODULE {
            continue;
        }
        if production_source(&path).contains("DROP TABLE") {
            offenders.push(name);
        }
    }
    assert!(
        offenders.is_empty(),
        "these modules drop their own tables instead of routing through \
         `crate::sidecar::open_sidecar`: {offenders:?}\n\n\
         A hand-rolled open does not violate the upgrade policy — it never reaches it, \
         which is why no behavioural test catches the omission. If the store holds \
         authored work, `Durability::UserAuthored` renames it aside on a bump and \
         refuses a downgrade; if it holds only derived values, `Durability::Cache` \
         rebuilds it empty exactly as before. Both are one call."
    );
}

/// Every module that DECLARES a sidecar schema version must also classify it.
///
/// The complement of the check above: dropping is how the old destructive open was
/// spelled, but a future store could hand-roll an open that never drops and still sit
/// outside the policy — so the trigger here is declaring a version at all.
#[test]
fn every_sidecar_schema_version_is_classified() {
    let mut unclassified: Vec<String> = Vec::new();
    for path in rust_sources() {
        let name = file_name(&path);
        if name == POLICY_MODULE {
            continue;
        }
        let src = production_source(&path);
        let declares_version = src.lines().any(|l| {
            let t = l.trim_start();
            (t.starts_with("const ")
                || t.starts_with("pub const ")
                || t.starts_with("pub(crate) const "))
                && t.contains("SCHEMA_VERSION")
                && t.contains('=')
        });
        if declares_version && !src.contains("sidecar::open_sidecar") {
            unclassified.push(name);
        }
    }
    assert!(
        unclassified.is_empty(),
        "these modules declare a sidecar SCHEMA_VERSION but never call \
         `crate::sidecar::open_sidecar`, so nothing decides what a bump does to their \
         data: {unclassified:?}\n\n\
         Classify the store — `UserAuthored` (rename aside + re-import; refuse a \
         downgrade) or `Cache` (rebuild empty) — and route the open through the policy."
    );
}

/// The classification is a real decision, so BOTH answers must be in use.
///
/// If every store were `Cache` the policy would be an elaborate way to spell the old
/// destructive behaviour, and this suite would still pass. Naming the stores that hold
/// authored work makes the loss of any one of them a visible edit rather than a silent
/// reclassification.
#[test]
fn the_stores_holding_authored_work_are_named_and_protected() {
    // Each of these holds something a user made and cannot regenerate from anything the
    // runtime still has. Removing a name from this list is the edit that would have to
    // be argued for.
    const AUTHORED: &[&str] = &[
        "apps.rs",
        "workflows.rs",
        "branches.rs",
        "triggers_store.rs",
        "skills.rs",
        "secrets.rs",
        // tools.db: registered tools AND every registered script.
        "tool_store.rs",
        // policies.db: durable Policy/Roles. Losing one does not lose a
        // capability — it RESTORES capability the operator meant to remove,
        // which is the failure direction that does not announce itself.
        "policies.rs",
    ];

    let present: BTreeSet<String> = rust_sources().iter().map(|p| file_name(p)).collect();
    for module in AUTHORED {
        assert!(
            present.contains(*module),
            "{module} is gone — if the store was renamed or removed, update this list \
             deliberately rather than letting the protection lapse silently"
        );
        let src = production_source(&gateway_src().join(module));
        assert!(
            src.contains("Durability::UserAuthored"),
            "{module} holds authored work and must open as \
             `Durability::UserAuthored`, so a schema bump renames its catalog aside \
             instead of dropping it. Found no such classification."
        );
        assert!(
            !src.contains("Durability::Cache"),
            "{module} is classified as a Cache somewhere — a store holding authored \
             work must never be rebuilt empty on a version bump"
        );
    }
}
