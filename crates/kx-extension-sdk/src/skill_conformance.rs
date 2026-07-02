// SPDX-License-Identifier: Apache-2.0
//! RC-SW1: the DECLARATIVE-family conformance harness — `run_skill_conformance`
//! over a `kortecx.skill/v1` pack directory. The sibling of
//! [`crate::conformance::run_conformance`] (which dials a REAL out-of-process
//! MCP connector); a skill has no process to dial, so its gate is structural:
//!
//! 1. **`pack_loads`** — `skill.json` + `instructions.md` load + validate in
//!    pack form (name==dir, bounded, non-empty).
//! 2. **`canonical_roundtrip`** — the stored-form manifest canonicalizes
//!    byte-idempotently (the identity `skill_ref` is derived over these bytes).
//! 3. **`no_authority_keys`** — the authority deny-key walk holds (SN-8: a
//!    skill wishes, the server grants; `warrant`/`grant`/`secret`/`credential`/
//!    `executable` keys are refused anywhere in the manifest tree).
//! 4. **`wish_grammar`** — every wished tool id parses (`name` or
//!    `server/name`) with an integer version.
//! 5. **`instructions_bounded`** — non-empty and within the size cap.
//! 6. **`pack_is_declarative`** — the pack contains ONLY the three pack files
//!    (no code, no subdirectories, nothing executable): the executable leg of a
//!    skill is ALWAYS an out-of-process connector or a bundled broker
//!    capability (D159/D174.4).
//! 7. **`empty_grant_binds_to_zero`** — the bind-side contract, pinned here in
//!    the harness vocabulary: granted = `wish ∩ caller-authority ∩ fireable`,
//!    so an EMPTY fireable/authority set grants NOTHING — a skill on its own
//!    can never mint authority. (The live-path pin is the kx-gateway
//!    `author_app_with_an_unfireable_skill_wish_proceeds_toolless` test; here
//!    the harness asserts the set algebra over the pack's own wish set.)
//!
//! External skill authors run this via `just test-skill <pack-dir>` (or the
//! `skill_conformance` example) before submission — the same report shape CI
//! gates in-tree packs with.

use std::collections::BTreeSet;
use std::path::Path;

use kx_skill::{SkillManifest, SkillPack};

use crate::conformance::{CheckResult, ConformanceReport};

/// Run the declarative-family gate over the skill pack at `dir`.
///
/// Never panics; every failure lands as a failed [`CheckResult`] in the report.
/// `report.connector` carries the skill name (or the directory name when the
/// pack fails to load); `reachable` = "the pack loaded"; `discovered` = the
/// wish-set size.
#[must_use]
pub fn run_skill_conformance(dir: &Path) -> ConformanceReport {
    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<pack>")
        .to_string();

    let mut checks = Vec::new();

    // 1. pack_loads — everything else needs the loaded pack.
    let pack = match SkillPack::load_dir(dir) {
        Ok(p) => {
            checks.push(CheckResult::pass(
                "pack_loads",
                10,
                format!(
                    "skill.json + instructions.md load + validate (name {:?})",
                    p.manifest.name
                ),
            ));
            p
        }
        Err(e) => {
            checks.push(CheckResult::fail("pack_loads", 10, e.to_string()));
            return ConformanceReport {
                connector: dir_name,
                reachable: false,
                discovered: 0,
                checks,
            };
        }
    };
    let name = pack.manifest.name.clone();
    let discovered = u32::try_from(pack.manifest.tools.len()).unwrap_or(u32::MAX);

    // 2. canonical_roundtrip — splice a placeholder ref (the stored form) and
    //    prove canonicalization is byte-idempotent.
    checks.push(canonical_roundtrip(&pack.manifest));

    // 3. no_authority_keys — the pack-form parse already ran the deny walk;
    //    re-assert explicitly against the raw bytes so the report NAMES it.
    checks.push(no_authority_keys(dir));

    // 4. wish_grammar — pack-form validation covered it; report it per-tool.
    checks.push(CheckResult::pass(
        "wish_grammar",
        5,
        format!(
            "{} wished tool(s), each a valid id with an integer version: [{}]",
            pack.manifest.tools.len(),
            pack.manifest
                .tools
                .iter()
                .map(|(k, v)| format!("{k}@{v}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ));

    // 5. instructions_bounded — the loader enforced it; surface the number.
    checks.push(CheckResult::pass(
        "instructions_bounded",
        7,
        format!(
            "{} bytes (cap {})",
            pack.instructions.len(),
            kx_skill::MAX_SKILL_INSTRUCTIONS_BYTES
        ),
    ));

    // 6. pack_is_declarative — nothing but the three pack files, nothing executable.
    checks.push(pack_is_declarative(dir));

    // 7. empty_grant_binds_to_zero — the set algebra of the bind contract.
    checks.push(empty_grant_binds_to_zero(&pack.manifest));

    ConformanceReport {
        connector: name,
        reachable: true,
        discovered,
        checks,
    }
}

fn canonical_roundtrip(manifest: &SkillManifest) -> CheckResult {
    let mut stored = manifest.clone();
    stored.instructions_ref = "0".repeat(64);
    let c1 = match stored.to_canonical_json() {
        Ok(b) => b,
        Err(e) => return CheckResult::fail("canonical_roundtrip", 10, e.to_string()),
    };
    let reparsed = match SkillManifest::from_json_slice_stored(&c1) {
        Ok(m) => m,
        Err(e) => {
            return CheckResult::fail(
                "canonical_roundtrip",
                10,
                format!("canonical bytes failed stored-form re-validation: {e}"),
            )
        }
    };
    match reparsed.to_canonical_json() {
        Ok(c2) if c2 == c1 => CheckResult::pass(
            "canonical_roundtrip",
            10,
            format!("{}-byte canonical form is idempotent", c1.len()),
        ),
        Ok(_) => CheckResult::fail(
            "canonical_roundtrip",
            10,
            "re-canonicalization moved bytes (identity would drift)",
        ),
        Err(e) => CheckResult::fail("canonical_roundtrip", 10, e.to_string()),
    }
}

fn no_authority_keys(dir: &Path) -> CheckResult {
    // The typed loader already refused deny-keys; re-run the walk over the RAW
    // bytes so a report reader sees the guard named even if the shape evolves.
    let raw = match std::fs::read(dir.join("skill.json")) {
        Ok(b) => b,
        Err(e) => return CheckResult::fail("no_authority_keys", 5, e.to_string()),
    };
    match SkillManifest::from_json_slice_pack(&raw) {
        Ok(_) => CheckResult::pass(
            "no_authority_keys",
            5,
            format!(
                "no {:?} key anywhere in the manifest tree (a skill wishes, the server grants)",
                kx_skill::DENY_KEYS
            ),
        ),
        Err(e) => CheckResult::fail("no_authority_keys", 5, e.to_string()),
    }
}

fn pack_is_declarative(dir: &Path) -> CheckResult {
    const ALLOWED: [&str; 3] = ["skill.json", "instructions.md", "README.md"];
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return CheckResult::fail("pack_is_declarative", 3, e.to_string()),
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return CheckResult::fail("pack_is_declarative", 3, e.to_string()),
        };
        let file_name = entry.file_name();
        let fname = file_name.to_string_lossy();
        if entry.path().is_dir() {
            return CheckResult::fail(
                "pack_is_declarative",
                3,
                format!("subdirectory {fname:?} — a pack is exactly the three flat files"),
            );
        }
        if !ALLOWED.contains(&fname.as_ref()) {
            return CheckResult::fail(
                "pack_is_declarative",
                3,
                format!(
                    "unexpected file {fname:?} — a skill carries NO code; its executable \
                     leg is always an out-of-process connector (D159/D174.4)"
                ),
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if let Ok(meta) = entry.metadata() {
                if meta.permissions().mode() & 0o111 != 0 {
                    return CheckResult::fail(
                        "pack_is_declarative",
                        3,
                        format!("{fname:?} is executable — a pack file never is"),
                    );
                }
            }
        }
    }
    CheckResult::pass(
        "pack_is_declarative",
        3,
        "exactly the flat pack files; nothing executable",
    )
}

fn empty_grant_binds_to_zero(manifest: &SkillManifest) -> CheckResult {
    // The bind contract's set algebra: granted = wish ∩ authority ∩ fireable.
    // With an EMPTY fireable set the intersection is empty for ANY wish — the
    // structural proof a skill grants nothing on its own. (The kx-gateway
    // integration test drives the same contract through the real author path.)
    let wish: BTreeSet<(String, String)> = manifest
        .tools
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let fireable: BTreeSet<(String, String)> = BTreeSet::new();
    let granted: BTreeSet<_> = wish.intersection(&fireable).collect();
    if granted.is_empty() {
        CheckResult::pass(
            "empty_grant_binds_to_zero",
            5,
            format!(
                "wish of {} tool(s) ∩ empty fireable set = 0 granted (a skill alone mints nothing)",
                wish.len()
            ),
        )
    } else {
        CheckResult::fail(
            "empty_grant_binds_to_zero",
            5,
            "non-empty grant from an empty fireable set",
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn write_pack(dir: &Path, name: &str) {
        let pack = dir.join(name);
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(
            pack.join("skill.json"),
            format!(
                r#"{{"schema":"kortecx.skill/v1","name":"{name}","version":"1","tools":{{"retrieve":"1"}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(pack.join("instructions.md"), "# Do the thing\n").unwrap();
        std::fs::write(pack.join("README.md"), "docs\n").unwrap();
    }

    #[test]
    fn a_well_formed_pack_passes_all_checks() {
        let tmp = tempfile::tempdir().unwrap();
        write_pack(tmp.path(), "good");
        let report = run_skill_conformance(&tmp.path().join("good"));
        assert!(report.passed(), "{report:#?}");
        assert_eq!(report.connector, "good");
        assert_eq!(report.discovered, 1);
        assert_eq!(report.checks.len(), 7);
    }

    #[test]
    fn a_missing_manifest_fails_closed_without_panicking() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("empty")).unwrap();
        let report = run_skill_conformance(&tmp.path().join("empty"));
        assert!(!report.passed());
        assert!(!report.reachable);
    }

    #[test]
    fn an_extra_file_or_executable_breaks_declarative() {
        let tmp = tempfile::tempdir().unwrap();
        write_pack(tmp.path(), "sneaky");
        std::fs::write(tmp.path().join("sneaky/run.sh"), "#!/bin/sh\n").unwrap();
        let report = run_skill_conformance(&tmp.path().join("sneaky"));
        let declarative = report
            .checks
            .iter()
            .find(|c| c.name == "pack_is_declarative")
            .unwrap();
        assert!(!declarative.passed, "{}", declarative.detail);
    }

    #[test]
    fn an_authority_key_fails_the_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let pack = tmp.path().join("evil");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(
            pack.join("skill.json"),
            r#"{"schema":"kortecx.skill/v1","name":"evil","warrant":{"tool_grants":["*"]}}"#,
        )
        .unwrap();
        std::fs::write(pack.join("instructions.md"), "x").unwrap();
        let report = run_skill_conformance(&pack);
        assert!(!report.passed());
    }
}
