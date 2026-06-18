//! `kx context add | list | get | remove` — author + govern PR-7 context bundles
//! (named, content-addressed collections a caller attaches to a run via
//! `kx invoke --context <handle>`). Tri-surface parity with the SDK + UI.
//!
//! The bundle manifest lives in an off-journal `bundles.db` sidecar; the server
//! derives `bundle_ref` (SN-8) and scopes every bundle to the authoring party.
//! `--item <name>=<hex32>` attaches an existing content-store ref; `--file
//! <name>=<path>` uploads the file first (`PutContent`) then attaches its
//! server-derived ref.

use std::path::PathBuf;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `context` subcommand.
#[derive(Debug)]
pub enum ContextSub {
    /// Author (upsert) a bundle at a handle.
    Add(AddSpec),
    /// List the caller's bundles.
    List,
    /// Show one bundle's manifest.
    Get {
        /// The bundle handle.
        handle: String,
    },
    /// Unbind a bundle (its CAS blobs stay).
    Remove {
        /// The bundle handle.
        handle: String,
    },
}

/// A `context add` request, assembled from the flags.
#[derive(Debug)]
pub struct AddSpec {
    /// The `namespace/collection/name` AssetPath handle (upsert key).
    pub handle: String,
    /// Advisory description (never parsed for enforcement).
    pub description: String,
    /// Items already in the content store: `(name, 32B ref)`.
    pub refs: Vec<(String, [u8; 32])>,
    /// Items to upload first then attach: `(name, file path)`.
    pub files: Vec<(String, PathBuf)>,
}

/// Parsed `context` arguments.
#[derive(Debug)]
pub struct ContextArgs {
    /// The subcommand.
    pub sub: ContextSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Split a `name=value` flag value (the shape of `--item` / `--file`).
fn split_named(value: &str, flag: &str) -> Result<(String, String), CliError> {
    let (name, rest) = value
        .split_once('=')
        .ok_or_else(|| CliError::Usage(format!("{flag} expects <name>=<value>, got {value:?}")))?;
    if name.is_empty() || rest.is_empty() {
        return Err(CliError::Usage(format!(
            "{flag} requires a non-empty name and value"
        )));
    }
    Ok((name.to_string(), rest.to_string()))
}

/// Parse `context` args (the verb already consumed). The first token selects the
/// subcommand (`add` / `list` / `get` / `remove`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ContextArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("context requires a subcommand: add | list | get | remove".into())
    })?;

    let mut handle: Option<String> = None;
    let mut description = String::new();
    let mut refs: Vec<(String, [u8; 32])> = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--description" => description = next_value(&mut args, "--description")?,
            "--item" => {
                let (name, hexref) = split_named(&next_value(&mut args, "--item")?, "--item")?;
                let r = crate::hex::decode_fixed::<32>(&hexref)?;
                refs.push((name, r));
            }
            "--file" => {
                let (name, path) = split_named(&next_value(&mut args, "--file")?, "--file")?;
                files.push((name, PathBuf::from(path)));
            }
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
                "context {verb} requires a <handle> (namespace/collection/name)"
            ))
        })
    };

    let sub = match kw.as_str() {
        "list" => ContextSub::List,
        "get" => ContextSub::Get {
            handle: require_handle(handle, "get")?,
        },
        "remove" => ContextSub::Remove {
            handle: require_handle(handle, "remove")?,
        },
        "add" => {
            let handle = require_handle(handle, "add")?;
            if refs.is_empty() && files.is_empty() {
                return Err(CliError::Usage(
                    "context add requires at least one --item <name>=<hex32> or --file <name>=<path>".into(),
                ));
            }
            ContextSub::Add(AddSpec {
                handle,
                description,
                refs,
                files,
            })
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown context subcommand {other:?} (expected add | list | get | remove)"
            )))
        }
    };
    Ok(ContextArgs { sub, common })
}

/// Execute `context`.
pub async fn execute(args: ContextArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        ContextSub::Add(spec) => {
            let mut items: Vec<proto::ContextItem> = Vec::new();
            // Existing content-store refs.
            for (name, r) in spec.refs {
                items.push(proto::ContextItem {
                    name,
                    content_ref: r.to_vec(),
                    media_type: String::new(),
                });
            }
            // Files: upload each first (PutContent), then attach its server-derived ref.
            for (name, path) in spec.files {
                let payload = std::fs::read(&path)
                    .map_err(|e| CliError::Io(format!("read {}: {e}", path.display())))?;
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let put = client
                    .put_content(resolved.request(proto::PutContentRequest {
                        payload,
                        media_type: String::new(),
                        filename,
                    })?)
                    .await
                    .map_err(CliError::from_status)?
                    .into_inner();
                items.push(proto::ContextItem {
                    name,
                    content_ref: put.content_ref,
                    media_type: String::new(),
                });
            }
            let resp = client
                .put_context_bundle(resolved.request(proto::PutContextBundleRequest {
                    handle: spec.handle,
                    description: spec.description,
                    items,
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_put_context_bundle(&resp, json));
        }
        ContextSub::List => {
            let resp = client
                .list_context_bundles(resolved.request(proto::ListContextBundlesRequest {
                    limit: 0,
                    after_handle: String::new(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_context_bundles_list(&resp, json));
        }
        ContextSub::Get { handle } => {
            let resp = client
                .get_context_bundle(resolved.request(proto::GetContextBundleRequest { handle })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_get_context_bundle(&resp, json));
        }
        ContextSub::Remove { handle } => {
            let resp = client
                .delete_context_bundle(
                    resolved.request(proto::DeleteContextBundleRequest { handle })?,
                )
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_delete_context_bundle(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ContextArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_add_with_item_ref() {
        let r = "ab".repeat(32);
        let a = p(&["add", "team/ctx/notes", "--item", &format!("intro={r}")]).unwrap();
        let ContextSub::Add(spec) = a.sub else {
            panic!("expected Add");
        };
        assert_eq!(spec.handle, "team/ctx/notes");
        assert_eq!(spec.refs.len(), 1);
        assert_eq!(spec.refs[0].0, "intro");
        assert_eq!(spec.refs[0].1, [0xab; 32]);
    }

    #[test]
    fn parses_add_with_file_and_description() {
        let a = p(&[
            "add",
            "team/ctx/docs",
            "--file",
            "spec=/tmp/spec.md",
            "--description",
            "the spec",
        ])
        .unwrap();
        let ContextSub::Add(spec) = a.sub else {
            panic!("expected Add");
        };
        assert_eq!(spec.files.len(), 1);
        assert_eq!(spec.files[0].0, "spec");
        assert_eq!(spec.description, "the spec");
    }

    #[test]
    fn parses_list_get_remove() {
        assert!(matches!(p(&["list"]).unwrap().sub, ContextSub::List));
        assert!(matches!(
            p(&["get", "a/b/c"]).unwrap().sub,
            ContextSub::Get { .. }
        ));
        assert!(matches!(
            p(&["remove", "a/b/c"]).unwrap().sub,
            ContextSub::Remove { .. }
        ));
    }

    #[test]
    fn rejects_bad_inputs() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["add", "a/b/c"]).is_err(), "add needs an item");
        assert!(p(&["get"]).is_err(), "get needs a handle");
        assert!(
            p(&["add", "a/b/c", "--item", "noequals"]).is_err(),
            "item needs name=value"
        );
        assert!(
            p(&["add", "a/b/c", "--item", "x=nothex"]).is_err(),
            "item ref must be hex32"
        );
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
    }
}
