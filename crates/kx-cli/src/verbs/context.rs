//! `kx context add | list | get | edit | remove-item | describe | remove` —
//! author + govern + EDIT PR-7 context bundles (named, content-addressed
//! collections a caller attaches to a run via `kx invoke --context <handle>`).
//! Tri-surface parity with the SDK + UI.
//!
//! The bundle manifest lives in an off-journal `bundles.db` sidecar; the server
//! derives `bundle_ref` (SN-8) and scopes every bundle to the authoring party.
//! `--item <name>=<hex32>` attaches an existing content-store ref; `--file
//! <name>=<path>` uploads the file first (`PutContent`) then attaches its
//! server-derived ref.
//!
//! POC-2 context-edit: because the content store is IMMUTABLE, an item edit is a
//! pure CLIENT compose over existing RPCs — `GetContextBundle` → `PutContent`
//! (new bytes ⇒ a NEW ref) → `PutContextBundle` re-upsert with that item
//! re-pointed. `get --output` exports each item body; `edit` replaces/renames one
//! item; `remove-item` drops one; `describe` re-sets the description. All
//! off-journal ⇒ digest-invariant by construction.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

use crate::client::{next_value, ClientCommon, Resolved};
use crate::error::CliError;
use crate::format;

/// The `context` subcommand.
#[derive(Debug)]
pub enum ContextSub {
    /// Author (upsert) a bundle at a handle.
    Add(AddSpec),
    /// List the caller's bundles.
    List,
    /// Show one bundle's manifest, or `--output <dir>` to export each item body.
    Get {
        /// The bundle handle.
        handle: String,
        /// Export each item's body to this directory (+ a `manifest.json`).
        output: Option<PathBuf>,
    },
    /// Replace one item's body (POC-2) — upload `--file` and re-point the item,
    /// optionally renaming it with `--name`.
    Edit(EditSpec),
    /// Drop one item from a bundle (re-upsert the remainder).
    RemoveItem {
        /// The bundle handle.
        handle: String,
        /// Which item to drop.
        selector: ItemSelector,
    },
    /// Re-set a bundle's advisory description (re-upsert, items unchanged).
    Describe {
        /// The bundle handle.
        handle: String,
        /// The new description.
        description: String,
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

/// A `context edit` request (replace one item's body, optionally rename it).
#[derive(Debug)]
pub struct EditSpec {
    /// The bundle handle.
    pub handle: String,
    /// Which item to replace.
    pub selector: ItemSelector,
    /// The file whose bytes become the item's new body.
    pub file: PathBuf,
    /// An optional new advisory name for the edited item.
    pub new_name: Option<String>,
}

/// How a per-item verb selects its target: by advisory NAME or by 0-based INDEX.
/// A name with more than one match is ambiguous — pass `--index`.
#[derive(Debug, Clone)]
pub enum ItemSelector {
    /// The advisory item name.
    Name(String),
    /// The 0-based item index.
    Index(usize),
}

/// Parsed `context` arguments.
#[derive(Debug)]
pub struct ContextArgs {
    /// The subcommand.
    pub sub: ContextSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Split a `name=value` flag value (the shape of `add`'s `--item` / `--file`).
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
/// subcommand (`add` / `list` / `get` / `edit` / `remove-item` / `describe` /
/// `remove`). The overloaded `--item` / `--file` flags are interpreted by verb:
/// `add` uses `<name>=<value>` pairs; the per-item edit verbs use a bare selector
/// name (`--item`) and a single path (`--file`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ContextArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage(
            "context requires a subcommand: add | list | get | edit | remove-item | describe | remove"
                .into(),
        )
    })?;
    let is_add = kw == "add";

    let mut handle: Option<String> = None;
    let mut description: Option<String> = None;
    let mut refs: Vec<(String, [u8; 32])> = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut output: Option<PathBuf> = None;
    let mut item_name: Option<String> = None;
    let mut index: Option<usize> = None;
    let mut edit_file: Option<PathBuf> = None;
    let mut new_name: Option<String> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--description" => description = Some(next_value(&mut args, "--description")?),
            "--output" => output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            "--name" => new_name = Some(next_value(&mut args, "--name")?),
            "--index" => {
                let raw = next_value(&mut args, "--index")?;
                index = Some(raw.parse::<usize>().map_err(|_| {
                    CliError::Usage(format!("--index expects an integer, got {raw:?}"))
                })?);
            }
            "--item" if is_add => {
                let (name, hexref) = split_named(&next_value(&mut args, "--item")?, "--item")?;
                let r = crate::hex::decode_fixed::<32>(&hexref)?;
                refs.push((name, r));
            }
            "--item" => item_name = Some(next_value(&mut args, "--item")?),
            "--file" if is_add => {
                let (name, path) = split_named(&next_value(&mut args, "--file")?, "--file")?;
                files.push((name, PathBuf::from(path)));
            }
            "--file" => edit_file = Some(PathBuf::from(next_value(&mut args, "--file")?)),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            other if handle.is_none() => handle = Some(other.to_string()),
            other => return Err(CliError::Usage(format!("unexpected argument {other:?}"))),
        }
    }

    let sub = assemble_sub(
        &kw,
        Flags {
            handle,
            description,
            refs,
            files,
            output,
            item_name,
            index,
            edit_file,
            new_name,
        },
    )?;
    Ok(ContextArgs { sub, common })
}

/// The accumulated `context` flags, dispatched to a subcommand by [`assemble_sub`].
struct Flags {
    handle: Option<String>,
    description: Option<String>,
    refs: Vec<(String, [u8; 32])>,
    files: Vec<(String, PathBuf)>,
    output: Option<PathBuf>,
    item_name: Option<String>,
    index: Option<usize>,
    edit_file: Option<PathBuf>,
    new_name: Option<String>,
}

/// Validate the accumulated flags against the verb and build the subcommand.
fn assemble_sub(kw: &str, f: Flags) -> Result<ContextSub, CliError> {
    let Flags {
        handle,
        description,
        refs,
        files,
        output,
        item_name,
        index,
        edit_file,
        new_name,
    } = f;
    let require_handle = |h: Option<String>, verb: &str| -> Result<String, CliError> {
        h.filter(|s| !s.is_empty()).ok_or_else(|| {
            CliError::Usage(format!(
                "context {verb} requires a <handle> (namespace/collection/name)"
            ))
        })
    };
    // A per-item verb needs exactly one of `--item <name>` / `--index <n>`.
    let require_selector = |verb: &str| -> Result<ItemSelector, CliError> {
        match (&item_name, index) {
            (Some(_), Some(_)) => Err(CliError::Usage(format!(
                "context {verb}: pass either --item <name> OR --index <n>, not both"
            ))),
            (Some(name), None) => Ok(ItemSelector::Name(name.clone())),
            (None, Some(i)) => Ok(ItemSelector::Index(i)),
            (None, None) => Err(CliError::Usage(format!(
                "context {verb} requires --item <name> or --index <n>"
            ))),
        }
    };

    match kw {
        "list" => Ok(ContextSub::List),
        "get" => Ok(ContextSub::Get {
            handle: require_handle(handle, "get")?,
            output,
        }),
        "remove" => Ok(ContextSub::Remove {
            handle: require_handle(handle, "remove")?,
        }),
        "describe" => {
            let handle = require_handle(handle, "describe")?;
            Ok(ContextSub::Describe {
                handle,
                description: description.ok_or_else(|| {
                    CliError::Usage("context describe requires --description <text>".into())
                })?,
            })
        }
        "remove-item" => {
            let handle = require_handle(handle, "remove-item")?;
            Ok(ContextSub::RemoveItem {
                handle,
                selector: require_selector("remove-item")?,
            })
        }
        "edit" => {
            let handle = require_handle(handle, "edit")?;
            let selector = require_selector("edit")?;
            let file = edit_file.ok_or_else(|| {
                CliError::Usage("context edit requires --file <path> (the new body)".into())
            })?;
            Ok(ContextSub::Edit(EditSpec {
                handle,
                selector,
                file,
                new_name,
            }))
        }
        "add" => {
            let handle = require_handle(handle, "add")?;
            if refs.is_empty() && files.is_empty() {
                return Err(CliError::Usage(
                    "context add requires at least one --item <name>=<hex32> or --file <name>=<path>".into(),
                ));
            }
            Ok(ContextSub::Add(AddSpec {
                handle,
                description: description.unwrap_or_default(),
                refs,
                files,
            }))
        }
        other => Err(CliError::Usage(format!(
            "unknown context subcommand {other:?} (expected add | list | get | edit | remove-item | describe | remove)"
        ))),
    }
}

/// Resolve a selector against a fetched bundle's items → `(index, item)`.
/// A name with more than one match is AMBIGUOUS (pass `--index`); an unknown
/// name or an out-of-range index is a usage error.
fn resolve_item<'a>(
    items: &'a [proto::ContextItem],
    selector: &ItemSelector,
    handle: &str,
) -> Result<(usize, &'a proto::ContextItem), CliError> {
    match selector {
        ItemSelector::Index(i) => items.get(*i).map(|it| (*i, it)).ok_or_else(|| {
            CliError::Usage(format!("item index {i} is out of range for {handle:?}"))
        }),
        ItemSelector::Name(name) => {
            let matches: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(_, it)| &it.name == name)
                .map(|(i, _)| i)
                .collect();
            match matches.as_slice() {
                [] => Err(CliError::Usage(format!(
                    "no item named {name:?} in {handle:?}"
                ))),
                [i] => Ok((*i, &items[*i])),
                _ => Err(CliError::Usage(format!(
                    "item name {name:?} is ambiguous in {handle:?} ({} matches) — pass --index",
                    matches.len()
                ))),
            }
        }
    }
}

/// Fetch a bundle's manifest or fail with a usage error (uniform not-found).
async fn fetch_bundle(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    handle: &str,
) -> Result<proto::ContextBundle, CliError> {
    let resp = client
        .get_context_bundle(resolved.request(proto::GetContextBundleRequest {
            handle: handle.to_string(),
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    resp.bundle
        .filter(|_| resp.found)
        .ok_or_else(|| CliError::Usage(format!("context bundle {handle:?} not found")))
}

/// Sanitise an advisory item name into a safe, relative export filename: keep
/// only the basename, strip path separators / `..` / leading dots, fall back to
/// `item-<n>`. (Item names are advisory and attacker-influenceable in a shared
/// store — never trust them as a path.)
fn safe_filename(name: &str, idx: usize) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').trim();
    if cleaned.is_empty() || cleaned == ".." {
        format!("item-{idx}")
    } else {
        cleaned.to_string()
    }
}

/// `context add`: author/upsert a bundle from `--item`/`--file` flags.
async fn execute_add(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    spec: AddSpec,
    json: bool,
) -> Result<(), CliError> {
    let mut items: Vec<proto::ContextItem> = Vec::new();
    for (name, r) in spec.refs {
        items.push(proto::ContextItem {
            name,
            content_ref: r.to_vec(),
            media_type: String::new(),
        });
    }
    for (name, path) in spec.files {
        let content_ref = upload_file(client, resolved, &path).await?;
        items.push(proto::ContextItem {
            name,
            content_ref,
            media_type: String::new(),
        });
    }
    let resp = reupsert(client, resolved, &spec.handle, spec.description, items).await?;
    println!("{}", format::render_put_context_bundle(&resp, json));
    Ok(())
}

/// `context edit`: replace one item's body (POC-2) — `GetContextBundle` →
/// `PutContent` (new ref) → `PutContextBundle` re-upsert, preserving the item's
/// media (and name unless `--name` renames it).
async fn execute_edit(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    spec: EditSpec,
    json: bool,
) -> Result<(), CliError> {
    let bundle = fetch_bundle(client, resolved, &spec.handle).await?;
    let (idx, _) = resolve_item(&bundle.items, &spec.selector, &spec.handle)?;
    let new_ref = upload_file(client, resolved, &spec.file).await?;
    let mut items = bundle.items;
    items[idx].content_ref = new_ref;
    if let Some(name) = spec.new_name {
        items[idx].name = name;
    }
    let resp = reupsert(client, resolved, &spec.handle, bundle.description, items).await?;
    println!("{}", format::render_put_context_bundle(&resp, json));
    Ok(())
}

/// Execute `context`.
pub async fn execute(args: ContextArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        ContextSub::Add(spec) => execute_add(&mut client, &resolved, spec, json).await?,
        ContextSub::Edit(spec) => execute_edit(&mut client, &resolved, spec, json).await?,
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
        ContextSub::Get { handle, output } => {
            if let Some(dir) = output {
                export_bundle(&mut client, &resolved, &handle, &dir, json).await?;
            } else {
                let resp = client
                    .get_context_bundle(
                        resolved.request(proto::GetContextBundleRequest { handle })?,
                    )
                    .await
                    .map_err(CliError::from_status)?
                    .into_inner();
                println!("{}", format::render_get_context_bundle(&resp, json));
            }
        }
        ContextSub::RemoveItem { handle, selector } => {
            let bundle = fetch_bundle(&mut client, &resolved, &handle).await?;
            let (idx, _) = resolve_item(&bundle.items, &selector, &handle)?;
            if bundle.items.len() <= 1 {
                return Err(CliError::Usage(format!(
                    "removing the last item would empty {handle:?}; use `context remove {handle}` to unbind the whole handle"
                )));
            }
            let mut items = bundle.items;
            items.remove(idx);
            let resp = reupsert(&mut client, &resolved, &handle, bundle.description, items).await?;
            println!("{}", format::render_put_context_bundle(&resp, json));
        }
        ContextSub::Describe {
            handle,
            description,
        } => {
            let bundle = fetch_bundle(&mut client, &resolved, &handle).await?;
            let resp = reupsert(&mut client, &resolved, &handle, description, bundle.items).await?;
            println!("{}", format::render_put_context_bundle(&resp, json));
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

/// `PutContent` a file's bytes, returning the server-derived ref.
async fn upload_file(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    path: &Path,
) -> Result<Vec<u8>, CliError> {
    let payload =
        std::fs::read(path).map_err(|e| CliError::Io(format!("read {}: {e}", path.display())))?;
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
    Ok(put.content_ref)
}

/// Re-upsert a bundle's manifest (the POC-2 edit primitive — off-journal).
async fn reupsert(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    handle: &str,
    description: String,
    items: Vec<proto::ContextItem>,
) -> Result<proto::PutContextBundleResponse, CliError> {
    Ok(client
        .put_context_bundle(resolved.request(proto::PutContextBundleRequest {
            handle: handle.to_string(),
            description,
            items,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner())
}

/// `context get --output <dir>`: write each item body (uploads scope, FULL bytes)
/// to a safe filename under `dir`, plus a `manifest.json` mapping names → refs.
async fn export_bundle(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    handle: &str,
    dir: &Path,
    json: bool,
) -> Result<(), CliError> {
    let bundle = fetch_bundle(client, resolved, handle).await?;
    std::fs::create_dir_all(dir)
        .map_err(|e| CliError::Io(format!("--output {}: {e}", dir.display())))?;
    let mut used: HashSet<String> = HashSet::new();
    let mut manifest_rows: Vec<serde_json::Value> = Vec::new();
    for (idx, item) in bundle.items.iter().enumerate() {
        let blob = client
            .get_content(resolved.request(proto::GetContentRequest {
                content_ref: item.content_ref.clone(),
                instance_id: Vec::new(), // uploads scope (FULL bytes, uncapped)
            })?)
            .await
            .map_err(CliError::from_status)?
            .into_inner();
        // De-collide sanitised filenames (`name`, `name-2`, `name-3`, …).
        let stem = safe_filename(&item.name, idx);
        let mut fname = stem.clone();
        let mut n = 2;
        while !used.insert(fname.clone()) {
            fname = format!("{stem}-{n}");
            n += 1;
        }
        let path = dir.join(&fname);
        std::fs::write(&path, &blob.payload)
            .map_err(|e| CliError::Io(format!("write {}: {e}", path.display())))?;
        manifest_rows.push(serde_json::json!({
            "name": item.name,
            "file": fname,
            "content_ref": crate::hex::encode(&item.content_ref),
            "media_type": item.media_type,
            "bytes": blob.payload.len(),
        }));
    }
    let manifest = serde_json::json!({
        "handle": bundle.handle,
        "bundle_ref": crate::hex::encode(&bundle.bundle_ref),
        "description": bundle.description,
        "items": manifest_rows,
    });
    let manifest_path = dir.join("manifest.json");
    std::fs::write(&manifest_path, format!("{manifest:#}\n"))
        .map_err(|e| CliError::Io(format!("write {}: {e}", manifest_path.display())))?;
    if json {
        println!("{manifest}");
    } else {
        println!(
            "exported {} item(s) of {} to {}",
            bundle.items.len(),
            handle,
            dir.display()
        );
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
            ContextSub::Get { output: None, .. }
        ));
        assert!(matches!(
            p(&["remove", "a/b/c"]).unwrap().sub,
            ContextSub::Remove { .. }
        ));
    }

    #[test]
    fn parses_get_with_output_export() {
        let a = p(&["get", "a/b/c", "--output", "/tmp/out"]).unwrap();
        let ContextSub::Get { handle, output } = a.sub else {
            panic!("expected Get");
        };
        assert_eq!(handle, "a/b/c");
        assert_eq!(output, Some(PathBuf::from("/tmp/out")));
    }

    #[test]
    fn parses_edit_by_name_and_index() {
        let a = p(&["edit", "a/b/c", "--item", "intro", "--file", "/tmp/new.md"]).unwrap();
        let ContextSub::Edit(spec) = a.sub else {
            panic!("expected Edit");
        };
        assert!(matches!(spec.selector, ItemSelector::Name(ref n) if n == "intro"));
        assert_eq!(spec.file, PathBuf::from("/tmp/new.md"));
        assert!(spec.new_name.is_none());

        let b = p(&[
            "edit", "a/b/c", "--index", "2", "--file", "/tmp/x", "--name", "renamed",
        ])
        .unwrap();
        let ContextSub::Edit(spec) = b.sub else {
            panic!("expected Edit");
        };
        assert!(matches!(spec.selector, ItemSelector::Index(2)));
        assert_eq!(spec.new_name.as_deref(), Some("renamed"));
    }

    #[test]
    fn parses_remove_item_and_describe() {
        let a = p(&["remove-item", "a/b/c", "--item", "old"]).unwrap();
        assert!(matches!(
            a.sub,
            ContextSub::RemoveItem {
                selector: ItemSelector::Name(_),
                ..
            }
        ));
        let b = p(&["describe", "a/b/c", "--description", "new desc"]).unwrap();
        let ContextSub::Describe { description, .. } = b.sub else {
            panic!("expected Describe");
        };
        assert_eq!(description, "new desc");
    }

    #[test]
    fn rejects_bad_inputs() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["add", "a/b/c"]).is_err(), "add needs an item");
        assert!(p(&["get"]).is_err(), "get needs a handle");
        assert!(
            p(&["add", "a/b/c", "--item", "noequals"]).is_err(),
            "add item needs name=value"
        );
        assert!(
            p(&["add", "a/b/c", "--item", "x=nothex"]).is_err(),
            "add item ref must be hex32"
        );
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
        assert!(
            p(&["edit", "a/b/c", "--file", "/tmp/x"]).is_err(),
            "edit needs a selector"
        );
        assert!(
            p(&["edit", "a/b/c", "--item", "x"]).is_err(),
            "edit needs --file"
        );
        assert!(
            p(&["edit", "a/b/c", "--item", "x", "--index", "0", "--file", "/tmp/x"]).is_err(),
            "edit rejects both selectors"
        );
        assert!(
            p(&["remove-item", "a/b/c"]).is_err(),
            "remove-item needs a selector"
        );
        assert!(
            p(&["describe", "a/b/c"]).is_err(),
            "describe needs --description"
        );
    }

    #[test]
    fn safe_filename_is_path_safe() {
        assert_eq!(safe_filename("../../etc/passwd", 0), "passwd");
        assert_eq!(safe_filename("a/b/c.txt", 0), "c.txt");
        assert_eq!(safe_filename("", 3), "item-3");
        assert_eq!(safe_filename("..", 1), "item-1");
        assert_eq!(safe_filename("weird*name?.md", 0), "weird_name_.md");
    }
}
