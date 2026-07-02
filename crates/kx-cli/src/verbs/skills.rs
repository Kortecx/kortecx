//! `kx skills add | list | show | remove` — govern the per-principal skill
//! catalog (RC-SW1) over the gateway RPCs (`AddSkill` / `ListSkills` /
//! `GetSkillForm` / `RemoveSkill`). Tri-surface parity with the UI + SDK.
//!
//! A skill is a DECLARATIVE `kortecx.skill/v1` bundle — instructions + a tool
//! grant-WISH set. Adding one grants nothing: at `kx app run` the server
//! intersects the wish against the caller's grants and the live broker
//! (`wish ∩ grants ∩ fireable`). SN-8: the server validates the manifest
//! fail-closed (authority deny-keys), stores the instructions body via the
//! content-write seam, and derives `skill_ref` — the CLI never sends identity.

use std::path::PathBuf;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `skills` subcommand.
#[derive(Debug)]
pub enum SkillsSub {
    /// Add (upsert) a skill from a pack dir or a manifest file.
    Add(AddSpec),
    /// List the caller's skills.
    List,
    /// Show one skill's form: summary + wish set (with the advisory
    /// `registered` bit) + the instructions preview.
    Show {
        /// The skill name.
        name: String,
    },
    /// Remove a skill from the catalog (the content-store body is untouched).
    Remove {
        /// The skill name.
        name: String,
    },
}

/// A `skills add` request, assembled from the flags.
#[derive(Debug)]
pub struct AddSpec {
    /// PACK form: a directory holding `skill.json` + `instructions.md`.
    pub dir: Option<PathBuf>,
    /// FILE form: the manifest JSON path (pack form with `--instructions`,
    /// stored form — a 64-hex `instructions_ref` inside — without).
    pub manifest: Option<PathBuf>,
    /// FILE form: the instructions markdown path.
    pub instructions: Option<PathBuf>,
}

/// Parsed `skills` arguments.
#[derive(Debug)]
pub struct SkillsArgs {
    /// The subcommand.
    pub sub: SkillsSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `skills` args (the verb already consumed). The first token selects the
/// subcommand (`add` / `list` / `show` / `remove`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<SkillsArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("skills requires a subcommand: add | list | show | remove".into())
    })?;

    let mut name: Option<String> = None;
    let mut dir: Option<PathBuf> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut instructions: Option<PathBuf> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--name" => name = Some(next_value(&mut args, "--name")?),
            "--dir" => dir = Some(PathBuf::from(next_value(&mut args, "--dir")?)),
            "--manifest" => manifest = Some(PathBuf::from(next_value(&mut args, "--manifest")?)),
            "--instructions" => {
                instructions = Some(PathBuf::from(next_value(&mut args, "--instructions")?));
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let require_name = |name: Option<String>, verb: &str| -> Result<String, CliError> {
        name.filter(|s| !s.is_empty())
            .ok_or_else(|| CliError::Usage(format!("skills {verb} requires --name <skill>")))
    };

    let sub = match kw.as_str() {
        "list" => SkillsSub::List,
        "show" => SkillsSub::Show {
            name: require_name(name, "show")?,
        },
        "remove" => SkillsSub::Remove {
            name: require_name(name, "remove")?,
        },
        "add" => {
            match (&dir, &manifest) {
                (Some(_), Some(_)) => {
                    return Err(CliError::Usage(
                        "skills add takes --dir <pack> OR --manifest <file>, not both".into(),
                    ))
                }
                (None, None) => {
                    return Err(CliError::Usage(
                        "skills add requires --dir <pack-dir> or --manifest <file>".into(),
                    ))
                }
                _ => {}
            }
            if dir.is_some() && instructions.is_some() {
                return Err(CliError::Usage(
                    "--instructions rides --manifest; a --dir pack carries its own instructions.md"
                        .into(),
                ));
            }
            SkillsSub::Add(AddSpec {
                dir,
                manifest,
                instructions,
            })
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown skills subcommand {other:?} (expected add | list | show | remove)"
            )))
        }
    };
    Ok(SkillsArgs { sub, common })
}

/// Resolve an [`AddSpec`] into the wire request — the PACK path validates
/// LOCALLY first (`kx-skill`, the same checks the server re-runs) so a bad pack
/// fails with the author-friendly error before any dial.
fn add_request(spec: &AddSpec) -> Result<proto::AddSkillRequest, CliError> {
    if let Some(dir) = &spec.dir {
        let pack = kx_skill::SkillPack::load_dir(dir)
            .map_err(|e| CliError::Usage(format!("skill pack {}: {e}", dir.display())))?;
        let manifest_json = pack
            .manifest
            .to_canonical_json()
            .map_err(|e| CliError::Usage(format!("skill pack {}: {e}", dir.display())))?;
        return Ok(proto::AddSkillRequest {
            manifest_json,
            instructions_body: pack.instructions.into_bytes(),
        });
    }
    let Some(manifest_path) = spec.manifest.as_ref() else {
        return Err(CliError::Usage(
            "skills add requires --dir <pack-dir> or --manifest <file>".into(),
        ));
    };
    let manifest_json = std::fs::read(manifest_path)
        .map_err(|e| CliError::Usage(format!("{}: {e}", manifest_path.display())))?;
    let instructions_body = match &spec.instructions {
        Some(p) => {
            std::fs::read(p).map_err(|e| CliError::Usage(format!("{}: {e}", p.display())))?
        }
        None => Vec::new(),
    };
    Ok(proto::AddSkillRequest {
        manifest_json,
        instructions_body,
    })
}

/// Execute `skills`.
pub async fn execute(args: SkillsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        SkillsSub::Add(spec) => {
            let req = add_request(&spec)?;
            let resp = client
                .add_skill(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_add_skill(&resp, json));
        }
        SkillsSub::List => {
            let req = proto::ListSkillsRequest {
                limit: 0,
                after_name: String::new(),
            };
            let resp = client
                .list_skills(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_skills_list(&resp, json));
        }
        SkillsSub::Show { name } => {
            let req = proto::GetSkillFormRequest { name };
            let resp = client
                .get_skill_form(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_skill_form(&resp, json));
        }
        SkillsSub::Remove { name } => {
            let req = proto::RemoveSkillRequest { name };
            let resp = client
                .remove_skill(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_remove_skill(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<SkillsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_the_four_subcommands() {
        assert!(matches!(
            p(&["add", "--dir", "skills/email-triage"]).unwrap().sub,
            SkillsSub::Add(AddSpec { dir: Some(_), .. })
        ));
        assert!(matches!(p(&["list"]).unwrap().sub, SkillsSub::List));
        assert!(matches!(
            p(&["show", "--name", "x"]).unwrap().sub,
            SkillsSub::Show { .. }
        ));
        assert!(matches!(
            p(&["remove", "--name", "x"]).unwrap().sub,
            SkillsSub::Remove { .. }
        ));
    }

    #[test]
    fn add_requires_exactly_one_source_and_instructions_only_with_manifest() {
        assert!(p(&["add"]).is_err(), "add needs a source");
        assert!(
            p(&["add", "--dir", "d", "--manifest", "m.json"]).is_err(),
            "dir XOR manifest"
        );
        assert!(
            p(&["add", "--dir", "d", "--instructions", "i.md"]).is_err(),
            "--instructions rides --manifest"
        );
        assert!(p(&["add", "--manifest", "m.json", "--instructions", "i.md"]).is_ok());
        assert!(
            p(&["add", "--manifest", "m.json"]).is_ok(),
            "stored-form manifest needs no body"
        );
    }

    #[test]
    fn show_and_remove_require_a_name_and_unknowns_are_rejected() {
        assert!(p(&["show"]).is_err());
        assert!(p(&["remove"]).is_err());
        assert!(p(&["frobnicate"]).is_err());
        assert!(p(&[]).is_err());
    }

    #[test]
    fn a_reference_pack_resolves_to_a_pack_form_request() {
        // The in-tree reference pack loads + canonicalizes locally (the same
        // validation the server re-runs), and the body rides the request.
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/email-triage");
        let spec = AddSpec {
            dir: Some(dir),
            manifest: None,
            instructions: None,
        };
        let req = add_request(&spec).unwrap();
        assert!(!req.manifest_json.is_empty());
        assert!(!req.instructions_body.is_empty());
        let v: serde_json::Value = serde_json::from_slice(&req.manifest_json).unwrap();
        assert_eq!(v["name"], "email-triage");
        assert!(
            v.get("instructions_ref").is_none(),
            "pack form carries no ref (server-derived)"
        );
    }
}
