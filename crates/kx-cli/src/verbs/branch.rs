//! `kx branch create | snapshot | list | get | remove` — author + govern D155
//! branches (named, content-addressed `{path -> ContentRef}` manifests over
//! operator-approved host files). Tri-surface parity with the SDK + UI.
//!
//! A branch lives in an off-journal `branches.db` sidecar; the server derives
//! `branch_ref` (SN-8) and scopes every branch to the authoring party. `snapshot`
//! reads confined host files (under `KX_SERVE_FS_ROOT`, default-OFF) INTO the
//! content store and records the `{path -> ref}` manifest; the host is never
//! written (Phase-A). `create --parent` forks a point-in-time CoW sub-branch.

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;
use kx_proto::proto;

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
/// subcommand (`create` / `snapshot` / `list` / `get` / `remove`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<BranchArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage(
            "branch requires a subcommand: create | snapshot | list | get | remove".into(),
        )
    })?;

    let mut handle: Option<String> = None;
    let mut parent: Option<String> = None;
    let mut description = String::new();
    let mut paths: Vec<String> = Vec::new();
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--description" => description = next_value(&mut args, "--description")?,
            "--parent" => parent = Some(next_value(&mut args, "--parent")?),
            "--path" => paths.push(next_value(&mut args, "--path")?),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            other if handle.is_none() => handle = Some(other.to_string()),
            other => return Err(CliError::Usage(format!("unexpected argument {other:?}"))),
        }
    }

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
        other => {
            return Err(CliError::Usage(format!(
                "unknown branch subcommand {other:?} (expected create | snapshot | list | get | remove)"
            )))
        }
    };
    Ok(BranchArgs { sub, common })
}

/// Execute `branch`.
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
    }
    Ok(())
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
}
