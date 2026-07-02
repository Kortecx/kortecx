//! `kx new skill <name> [--dir <parent>]` — scaffold a `kortecx.skill/v1` pack
//! (RC-SW1, the D175 test-infra scaffolder). Fully OFFLINE (no gateway): emits
//! the three pack files with a template the author fills in, then points at the
//! conformance + add + registry steps.
//!
//! What it deliberately does NOT do: contact a gateway, derive refs/hashes, run
//! conformance, or edit `registry/index.json` / `feature-ledger.toml` — a skill
//! is declarative by construction; the emitted README carries the next-steps
//! checklist instead.

use std::path::PathBuf;

use crate::client::next_value;
use crate::error::CliError;

/// The `new` subcommand (`skill` today; `connector` is a later enabler).
#[derive(Debug)]
pub enum NewSub {
    /// Scaffold a skill pack directory.
    Skill {
        /// The skill (and directory) name — `[a-z0-9._-]{1,64}`.
        name: String,
        /// Parent directory (default `.`); the pack lands at `<dir>/<name>/`.
        dir: PathBuf,
    },
}

/// Parsed `new` arguments.
#[derive(Debug)]
pub struct NewArgs {
    /// The subcommand.
    pub sub: NewSub,
}

/// The scaffolded `skill.json` (pack form — no `instructions_ref`; the server
/// derives it at `kx skills add`). `__NAME__` is substituted.
const SKILL_JSON_TEMPLATE: &str = r#"{
  "schema": "kortecx.skill/v1",
  "name": "__NAME__",
  "version": "1",
  "description": "One sentence: what outcome this skill produces.",
  "tags": [],
  "tools": {}
}
"#;

const INSTRUCTIONS_TEMPLATE: &str = "# __NAME__

You are … (the role this skill gives the agent).

## Procedure

1. …the ordered steps; name each tool you expect to use and when.
2. …

## Boundaries

- …what this skill must never do (the wish set below enforces the hard line;
  write the soft lines here).

## Output contract

…the shape of the final answer the user should get.
";

const README_TEMPLATE: &str = "# __NAME__

A `kortecx.skill/v1` pack: declarative instructions + a tool grant-WISH set.
Attaching it grants nothing — at run the server intersects the wish against the
caller's grants and the live broker (`wish ∩ grants ∩ fireable`).

Fill in `skill.json` `tools` with the `(tool_id → version)` wishes, e.g.
`{\"retrieve\": \"1\", \"gmail/search\": \"1\"}` (a connector tool is
`<connection-name>/<tool>`), and write the instructions in `instructions.md`.

## Next steps

1. `just test-skill <this-dir>` — the declarative conformance gate (or
   `cargo run -p kx-extension-sdk --example skill_conformance -- <this-dir>`).
2. `kx skills add --dir <this-dir>` — add it to your serve's catalog.
3. `kx app new <app> --from-blueprint <bp.json> --skill __NAME__` — attach it.
4. Upstreaming in-tree? Add a `registry/index.json` entry + a
   `feature-ledger.toml` row (`just registry-check` verifies).
";

/// Parse `new` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<NewArgs, CliError> {
    let kind = args
        .next()
        .ok_or_else(|| CliError::Usage("new requires a kind: skill".into()))?;
    if kind != "skill" {
        return Err(CliError::Usage(format!(
            "unknown new kind {kind:?} (expected skill)"
        )));
    }
    let mut name: Option<String> = None;
    let mut dir = PathBuf::from(".");
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--dir" => dir = PathBuf::from(next_value(&mut args, "--dir")?),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            positional if name.is_none() => name = Some(positional.to_string()),
            extra => return Err(CliError::Usage(format!("unexpected argument {extra:?}"))),
        }
    }
    let name = name.ok_or_else(|| CliError::Usage("new skill requires <name>".into()))?;
    Ok(NewArgs {
        sub: NewSub::Skill { name, dir },
    })
}

/// Execute `new` (offline).
pub fn execute(args: NewArgs) -> Result<(), CliError> {
    let NewSub::Skill { name, dir } = args.sub;
    // The same grammar kx-skill enforces — fail here with the author-friendly
    // message instead of at add time.
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
    {
        return Err(CliError::Usage(format!(
            "skill name must be 1-64 chars of [a-z0-9._-], got {name:?}"
        )));
    }
    let pack = dir.join(&name);
    // Fail-closed: never clobber an existing non-empty directory.
    if pack.exists()
        && pack
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(CliError::Usage(format!(
            "{} already exists and is not empty",
            pack.display()
        )));
    }
    std::fs::create_dir_all(&pack)
        .map_err(|e| CliError::Usage(format!("{}: {e}", pack.display())))?;
    for (file, template) in [
        ("skill.json", SKILL_JSON_TEMPLATE),
        ("instructions.md", INSTRUCTIONS_TEMPLATE),
        ("README.md", README_TEMPLATE),
    ] {
        std::fs::write(pack.join(file), template.replace("__NAME__", &name))
            .map_err(|e| CliError::Usage(format!("{}/{file}: {e}", pack.display())))?;
    }
    println!(
        "scaffolded skill pack {}\n  next: edit skill.json (the tool wishes) + instructions.md, \
         then `just test-skill {}` and `kx skills add --dir {}`",
        pack.display(),
        pack.display(),
        pack.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_skill_with_a_dir() {
        let a = parse(
            ["skill", "triage", "--dir", "/tmp/x"]
                .iter()
                .map(|s| (*s).to_string()),
        )
        .unwrap();
        let NewSub::Skill { name, dir } = a.sub;
        assert_eq!(name, "triage");
        assert_eq!(dir, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn rejects_missing_name_bad_kind_and_unknown_flags() {
        let p = |parts: &[&str]| parse(parts.iter().map(|s| (*s).to_string()));
        assert!(p(&[]).is_err());
        assert!(
            p(&["connector", "x"]).is_err(),
            "connector is a later enabler"
        );
        assert!(p(&["skill"]).is_err(), "needs a name");
        assert!(p(&["skill", "x", "--frob", "y"]).is_err());
    }

    #[test]
    fn the_emitted_template_passes_pack_validation_after_a_wish_is_added() {
        // Template-drift pin: what `kx new skill` emits must load as a valid
        // pack (kx-skill is the SAME validator the server runs).
        let tmp = tempfile::tempdir().unwrap();
        execute(
            parse(
                ["skill", "my-skill", "--dir", tmp.path().to_str().unwrap()]
                    .iter()
                    .map(|s| (*s).to_string()),
            )
            .unwrap(),
        )
        .unwrap();
        let pack = kx_skill::SkillPack::load_dir(&tmp.path().join("my-skill")).unwrap();
        assert_eq!(pack.manifest.name, "my-skill");
        assert!(pack.manifest.tools.is_empty(), "wishes start empty");
        assert!(pack.readme.is_some());
    }

    #[test]
    fn refuses_a_non_empty_target_and_a_bad_name() {
        let tmp = tempfile::tempdir().unwrap();
        let run = |name: &str| {
            execute(NewArgs {
                sub: NewSub::Skill {
                    name: name.to_string(),
                    dir: tmp.path().to_path_buf(),
                },
            })
        };
        assert!(run("UPPER").is_err(), "grammar enforced offline");
        run("taken").unwrap();
        assert!(run("taken").is_err(), "never clobbers a non-empty pack");
    }
}
