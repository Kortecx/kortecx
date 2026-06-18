//! `kx branch create | snapshot | list | get | remove` — author + govern D155
//! branches (named, content-addressed `{path -> ContentRef}` manifests over
//! operator-approved host files). Tri-surface parity with the SDK + UI.
//!
//! A branch lives in an off-journal `branches.db` sidecar; the server derives
//! `branch_ref` (SN-8) and scopes every branch to the authoring party. `snapshot`
//! reads confined host files (under `KX_SERVE_FS_ROOT`, default-OFF) INTO the
//! content store and records the `{path -> ref}` manifest; the host is never
//! written (Phase-A). `create --parent` forks a point-in-time CoW sub-branch.

use std::time::Duration;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::{format, hex, wait};
use kx_proto::proto;

/// Default `--timeout-secs` for `branch edit` (a model rewrite over a possibly-
/// large file ⇒ generous; CPU/Metal inference is slow).
const DEFAULT_EDIT_TIMEOUT_SECS: u64 = 300;

/// The `branch` subcommand.
#[derive(Debug)]
pub enum BranchSub {
    /// Create (or fork via `--parent`) a branch.
    Create {
        /// The `namespace/collection/name` AssetPath handle.
        handle: String,
        /// Optional CoW parent handle (a point-in-time fork).
        parent: Option<String>,
        /// Advisory description.
        description: String,
    },
    /// Snapshot host files into a branch (creating it if absent).
    Snapshot {
        /// The branch handle.
        handle: String,
        /// The host paths (confined under `KX_SERVE_FS_ROOT`) to read into CAS.
        paths: Vec<String>,
        /// Optional CoW parent handle (applied iff the branch is created).
        parent: Option<String>,
        /// Advisory description (applied iff the branch is created).
        description: String,
    },
    /// List the caller's branches.
    List,
    /// Show one branch's resolved manifest.
    Get {
        /// The branch handle.
        handle: String,
    },
    /// Unbind a branch (its CAS blobs stay).
    Remove {
        /// The branch handle.
        handle: String,
    },
    /// D155 Phase-3: agentically edit a branch file IN-CAS. A single model step
    /// rewrites the file's current body (attached as a context ref) per
    /// `instruction`; the edited body commits as a NEW content ref and the
    /// manifest advances to it. The host is NEVER written.
    Edit {
        /// The branch handle.
        handle: String,
        /// The manifest path to edit (must already be in the branch).
        path: String,
        /// The edit instruction for the model.
        instruction: String,
        /// `--wait` timeout (seconds) for the rewrite.
        timeout_secs: u64,
    },
    /// D155 Phase-3 (low-level): re-point a branch `path` to an EXISTING content
    /// ref (the `AdvanceBranch` RPC). Power-user / scripting — `edit` is the
    /// agentic high-level verb.
    Advance {
        /// The branch handle.
        handle: String,
        /// The manifest path to re-point (or insert — "enrich").
        path: String,
        /// The 64-hex content-store ref to point at.
        content_ref: String,
    },
}

/// Parsed `branch` arguments.
#[derive(Debug)]
pub struct BranchArgs {
    /// The subcommand.
    pub sub: BranchSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `branch` args (the verb already consumed). The first token selects the
/// subcommand (`create` / `snapshot` / `list` / `get` / `remove` / `edit` / `advance`).
#[allow(clippy::too_many_lines)]
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<BranchArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage(
            "branch requires a subcommand: create | snapshot | list | get | remove | edit | advance"
                .into(),
        )
    })?;

    let mut handle: Option<String> = None;
    let mut parent: Option<String> = None;
    let mut description = String::new();
    let mut paths: Vec<String> = Vec::new();
    let mut instruction: Option<String> = None;
    let mut content_ref: Option<String> = None;
    let mut timeout_secs = DEFAULT_EDIT_TIMEOUT_SECS;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--description" => description = next_value(&mut args, "--description")?,
            "--parent" => parent = Some(next_value(&mut args, "--parent")?),
            "--path" => paths.push(next_value(&mut args, "--path")?),
            "--instruction" => instruction = Some(next_value(&mut args, "--instruction")?),
            "--ref" => content_ref = Some(next_value(&mut args, "--ref")?),
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            other if handle.is_none() => handle = Some(other.to_string()),
            other => return Err(CliError::Usage(format!("unexpected argument {other:?}"))),
        }
    }

    // `edit` / `advance` take a single `--path`.
    let single_path = |verb: &str| -> Result<String, CliError> {
        match paths.as_slice() {
            [p] => Ok(p.clone()),
            [] => Err(CliError::Usage(format!(
                "branch {verb} requires exactly one --path <subpath>"
            ))),
            _ => Err(CliError::Usage(format!(
                "branch {verb} takes exactly one --path <subpath>"
            ))),
        }
    };

    let require_handle = |h: Option<String>, verb: &str| -> Result<String, CliError> {
        h.filter(|s| !s.is_empty()).ok_or_else(|| {
            CliError::Usage(format!(
                "branch {verb} requires a <handle> (namespace/collection/name)"
            ))
        })
    };

    let sub = match kw.as_str() {
        "list" => BranchSub::List,
        "get" => BranchSub::Get {
            handle: require_handle(handle, "get")?,
        },
        "remove" => BranchSub::Remove {
            handle: require_handle(handle, "remove")?,
        },
        "create" => BranchSub::Create {
            handle: require_handle(handle, "create")?,
            parent,
            description,
        },
        "snapshot" => {
            let handle = require_handle(handle, "snapshot")?;
            if paths.is_empty() {
                return Err(CliError::Usage(
                    "branch snapshot requires at least one --path <subpath>".into(),
                ));
            }
            BranchSub::Snapshot {
                handle,
                paths,
                parent,
                description,
            }
        }
        "edit" => BranchSub::Edit {
            handle: require_handle(handle, "edit")?,
            path: single_path("edit")?,
            instruction: instruction
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| CliError::Usage("branch edit requires --instruction <text>".into()))?,
            timeout_secs,
        },
        "advance" => BranchSub::Advance {
            handle: require_handle(handle, "advance")?,
            path: single_path("advance")?,
            content_ref: content_ref
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CliError::Usage("branch advance requires --ref <64-hex>".into()))?,
        },
        other => {
            return Err(CliError::Usage(format!(
                "unknown branch subcommand {other:?} (expected create | snapshot | list | get | remove | edit | advance)"
            )))
        }
    };
    Ok(BranchArgs { sub, common })
}

/// Execute `branch`.
#[allow(clippy::too_many_lines)]
pub async fn execute(args: BranchArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        BranchSub::Create {
            handle,
            parent,
            description,
        } => {
            let resp = client
                .create_branch(resolved.request(proto::CreateBranchRequest {
                    handle,
                    description,
                    parent_handle: parent.unwrap_or_default(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_create_branch(&resp, json));
        }
        BranchSub::Snapshot {
            handle,
            paths,
            parent,
            description,
        } => {
            let resp = client
                .snapshot_into(resolved.request(proto::SnapshotIntoRequest {
                    handle,
                    paths,
                    description,
                    parent_handle: parent.unwrap_or_default(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_snapshot_into(&resp, json));
        }
        BranchSub::List => {
            let resp = client
                .list_branches(resolved.request(proto::ListBranchesRequest {
                    limit: 0,
                    after_handle: String::new(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_branches_list(&resp, json));
        }
        BranchSub::Get { handle } => {
            let resp = client
                .get_branch(resolved.request(proto::GetBranchRequest { handle })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_get_branch(&resp, json));
        }
        BranchSub::Remove { handle } => {
            let resp = client
                .delete_branch(resolved.request(proto::DeleteBranchRequest { handle })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_delete_branch(&resp, json));
        }
        BranchSub::Edit {
            handle,
            path,
            instruction,
            timeout_secs,
        } => {
            // (1) Resolve the file's CURRENT in-CAS ref from the manifest.
            let got = client
                .get_branch(resolved.request(proto::GetBranchRequest {
                    handle: handle.clone(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            let branch = got
                .branch
                .ok_or_else(|| CliError::Usage(format!("branch {handle:?} not found")))?;
            let item = branch
                .items
                .iter()
                .find(|it| it.path == path)
                .ok_or_else(|| {
                    CliError::Usage(format!("path {path:?} is not in branch {handle:?}"))
                })?;
            let cur_ref_hex = hex::encode(&item.content_ref);

            // (2) Invoke `react-edit` (a single model step) with the body attached
            //     as a context ref. The directive tells the model to emit ONLY the
            //     edited body (GR15 — no silent transform; reasoning=off keeps the
            //     committed answer the file verbatim).
            let args_json = edit_args_json(&path, &instruction);
            let resp = client
                .invoke(resolved.request(proto::InvokeRequest {
                    handle: kx_gateway::REACT_EDIT_RECIPE_HANDLE.to_string(),
                    args: args_json,
                    context_bundles: vec![],
                    context_refs: vec![cur_ref_hex],
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();

            // (3) The single step settles on its terminal mote → the edited body's
            //     committed result_ref (no react-chain turn dance).
            let outcome = wait::await_result(
                &mut client,
                &resolved,
                resp.instance_id,
                resp.terminal_mote_id,
                Duration::from_secs(timeout_secs),
            )
            .await?;
            let new_ref = outcome.result_ref.ok_or_else(|| {
                CliError::Runtime(format!(
                    "react-edit did not commit an answer to advance to (state: {:?})",
                    outcome.state
                ))
            })?;
            // Fail CLOSED on an empty edit (GR15): a heavy-reasoning model can emit
            // only an unclosed `<think>` block that strips to nothing — never
            // advance the manifest to an empty file. The branch is left unchanged.
            if outcome.payload.as_deref().is_none_or(<[u8]>::is_empty) {
                return Err(CliError::Runtime(
                    "react-edit produced an empty body (the model did not return file \
                     contents); the branch was NOT advanced. Try a model that completes \
                     the rewrite, or re-run."
                        .into(),
                ));
            }

            // (4) Advance the manifest (re-point the path to the new in-CAS body).
            let adv = client
                .advance_branch(resolved.request(proto::AdvanceBranchRequest {
                    handle,
                    path,
                    content_ref: new_ref,
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_advance_branch(&adv, json));
        }
        BranchSub::Advance {
            handle,
            path,
            content_ref,
        } => {
            let cref = hex::decode_fixed::<32>(&content_ref)?;
            let resp = client
                .advance_branch(resolved.request(proto::AdvanceBranchRequest {
                    handle,
                    path,
                    content_ref: cref.to_vec(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_advance_branch(&resp, json));
        }
    }
    Ok(())
}

/// Build the `react-edit` args JSON: the model's `prompt` is a directive to emit
/// ONLY the edited file body (GR15 — the committed answer is the new content
/// verbatim; reasoning=off, baked into the recipe, keeps it clean). The recipe is
/// a single model step, so the only free param is `prompt`.
fn edit_args_json(path: &str, instruction: &str) -> Vec<u8> {
    let directive = format!(
        "You are editing the file `{path}`. The text in the attached context below IS its exact \
         current contents. Apply this change: {instruction}\n\nReturn ONLY the complete, updated \
         file contents — no commentary, no explanation, and no markdown code fences.",
    );
    let obj = serde_json::json!({ "prompt": directive });
    serde_json::to_vec(&obj).unwrap_or_else(|_| b"{}".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<BranchArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_create_with_parent() {
        let a = p(&[
            "create",
            "team/br/feature",
            "--parent",
            "team/br/main",
            "--description",
            "a fork",
        ])
        .unwrap();
        let BranchSub::Create {
            handle,
            parent,
            description,
        } = a.sub
        else {
            panic!("expected Create");
        };
        assert_eq!(handle, "team/br/feature");
        assert_eq!(parent.as_deref(), Some("team/br/main"));
        assert_eq!(description, "a fork");
    }

    #[test]
    fn parses_snapshot_with_paths() {
        let a = p(&[
            "snapshot",
            "team/br/work",
            "--path",
            "src/lib.rs",
            "--path",
            "README.md",
        ])
        .unwrap();
        let BranchSub::Snapshot { handle, paths, .. } = a.sub else {
            panic!("expected Snapshot");
        };
        assert_eq!(handle, "team/br/work");
        assert_eq!(paths, vec!["src/lib.rs", "README.md"]);
    }

    #[test]
    fn parses_list_get_remove() {
        assert!(matches!(p(&["list"]).unwrap().sub, BranchSub::List));
        assert!(matches!(
            p(&["get", "a/b/c"]).unwrap().sub,
            BranchSub::Get { .. }
        ));
        assert!(matches!(
            p(&["remove", "a/b/c"]).unwrap().sub,
            BranchSub::Remove { .. }
        ));
    }

    #[test]
    fn rejects_bad_inputs() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["snapshot", "a/b/c"]).is_err(), "snapshot needs a path");
        assert!(p(&["get"]).is_err(), "get needs a handle");
        assert!(p(&["create"]).is_err(), "create needs a handle");
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
    }

    #[test]
    fn parses_edit_with_path_and_instruction() {
        let a = p(&[
            "edit",
            "team/br/work",
            "--path",
            "notes.md",
            "--instruction",
            "uppercase the title",
        ])
        .unwrap();
        let BranchSub::Edit {
            handle,
            path,
            instruction,
            ..
        } = a.sub
        else {
            panic!("expected Edit");
        };
        assert_eq!(handle, "team/br/work");
        assert_eq!(path, "notes.md");
        assert_eq!(instruction, "uppercase the title");
    }

    #[test]
    fn edit_rejects_missing_path_or_instruction() {
        assert!(
            p(&["edit", "a/b/c", "--instruction", "x"]).is_err(),
            "edit needs a --path"
        );
        assert!(
            p(&["edit", "a/b/c", "--path", "f.txt"]).is_err(),
            "edit needs an --instruction"
        );
        assert!(
            p(&[
                "edit",
                "a/b/c",
                "--path",
                "a",
                "--path",
                "b",
                "--instruction",
                "x"
            ])
            .is_err(),
            "edit takes exactly one --path"
        );
    }

    #[test]
    fn parses_advance_low_level() {
        let a = p(&[
            "advance",
            "team/br/work",
            "--path",
            "notes.md",
            "--ref",
            &"ab".repeat(32),
        ])
        .unwrap();
        let BranchSub::Advance {
            handle,
            path,
            content_ref,
        } = a.sub
        else {
            panic!("expected Advance");
        };
        assert_eq!(handle, "team/br/work");
        assert_eq!(path, "notes.md");
        assert_eq!(content_ref, "ab".repeat(32));
        // advance requires a --ref.
        assert!(p(&["advance", "a/b/c", "--path", "f.txt"]).is_err());
    }

    #[test]
    fn edit_directive_instructs_raw_body_no_fences() {
        let args = edit_args_json("notes.md", "uppercase the title");
        let v: serde_json::Value = serde_json::from_slice(&args).unwrap();
        // The single model step's only free param is `prompt` (the directive).
        let prompt = v["prompt"].as_str().unwrap();
        assert!(prompt.contains("notes.md"));
        assert!(prompt.contains("uppercase the title"));
        assert!(prompt.contains("no markdown code fences"));
        assert!(v.get("max_turns").is_none(), "single step has no caps");
    }
}
