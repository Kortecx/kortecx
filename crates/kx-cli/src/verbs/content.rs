//! `kx content` — fetch / upload content-store blobs.
//!
//! - `kx content put <file> [--media-type <mime>] [--filename <name>]` — Batch A
//!   client upload: a CONTENT-STORE write, never a journal write; the printed
//!   ref is SERVER-DERIVED blake3 (SN-8); the server caps the payload
//!   fail-closed (`kx serve --content-max-bytes`).
//! - `kx content get --ref <hex32> [--instance <hex16>] [--out <file>]` — fetch
//!   a blob. With `--instance` the run scope (the run's committed result refs);
//!   WITHOUT it the UPLOADS scope (refs this party uploaded). Denials are
//!   uniform (no existence oracle).
//! - The original flag-form `kx content --ref … --instance …` is preserved
//!   verbatim (back-compat: a first arg starting with `--` selects `get`).

use std::io::Write;
use std::path::PathBuf;

use kx_proto::proto;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// Parsed `content` arguments.
#[derive(Debug)]
pub enum ContentArgs {
    /// Fetch a blob (the original verb, now with an optional uploads scope).
    Get(GetArgs),
    /// Upload a file (Batch A `PutContent`).
    Put(PutArgs),
}

/// Parsed `content get` (and legacy flag-form) arguments.
#[derive(Debug)]
pub struct GetArgs {
    /// The content ref to fetch (32B).
    pub content_ref: [u8; 32],
    /// The owning run (16B instance id); `None` = the uploads scope (Batch A).
    pub instance: Option<[u8; 16]>,
    /// Write the bytes to this file instead of stdout.
    pub out: Option<PathBuf>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `content put` arguments.
#[derive(Debug)]
pub struct PutArgs {
    /// The file whose bytes to upload.
    pub file: PathBuf,
    /// Advisory mime (audit/display only; never identity).
    pub media_type: Option<String>,
    /// Advisory display name (defaults to the file's basename).
    pub filename: Option<String>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `content` args (the verb already consumed). A first token starting
/// with `--` is the LEGACY flag-form `get` (back-compat); `get` / `put` select
/// the subcommand explicitly.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ContentArgs, CliError> {
    let first = args.next().ok_or_else(|| {
        CliError::Usage("content requires a subcommand (get | put) or --ref …".into())
    })?;
    match first.as_str() {
        "put" => parse_put(args).map(ContentArgs::Put),
        "get" => parse_get(args, None).map(ContentArgs::Get),
        flag if flag.starts_with("--") => {
            // Legacy flag-form: re-feed the consumed flag into the get parser.
            parse_get(args, Some(first)).map(ContentArgs::Get)
        }
        other => Err(CliError::Usage(format!(
            "unknown content subcommand {other:?} (expected: get | put)"
        ))),
    }
}

fn parse_get(
    args: impl Iterator<Item = String>,
    consumed: Option<String>,
) -> Result<GetArgs, CliError> {
    let mut args = consumed.into_iter().chain(args);
    let mut content_ref: Option<[u8; 32]> = None;
    let mut instance: Option<[u8; 16]> = None;
    let mut out: Option<PathBuf> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--ref" => content_ref = Some(take_fixed::<_, 32>(&mut args, "--ref")?),
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let content_ref =
        content_ref.ok_or_else(|| CliError::Usage("content get requires --ref <hex32>".into()))?;
    Ok(GetArgs {
        content_ref,
        instance,
        out,
        common,
    })
}

fn parse_put(mut args: impl Iterator<Item = String>) -> Result<PutArgs, CliError> {
    let mut file: Option<PathBuf> = None;
    let mut media_type: Option<String> = None;
    let mut filename: Option<String> = None;
    let mut common = ClientCommon::default();

    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        match tok.as_str() {
            "--media-type" => media_type = Some(next_value(&mut args, "--media-type")?),
            "--filename" => filename = Some(next_value(&mut args, "--filename")?),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            _ if file.is_none() => file = Some(PathBuf::from(tok)),
            _ => {
                return Err(CliError::Usage(
                    "content put takes exactly one <file> argument".into(),
                ))
            }
        }
    }

    let file = file.ok_or_else(|| CliError::Usage("content put requires a <file>".into()))?;
    Ok(PutArgs {
        file,
        media_type,
        filename,
        common,
    })
}

/// Execute `content`.
pub async fn execute(args: ContentArgs) -> Result<(), CliError> {
    match args {
        ContentArgs::Get(a) => execute_get(a).await,
        ContentArgs::Put(a) => execute_put(a).await,
    }
}

async fn execute_get(args: GetArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    let blob = client
        .get_content(resolved.request(proto::GetContentRequest {
            content_ref: args.content_ref.to_vec(),
            // Empty = the uploads scope (Batch A); 16B = the run ticket.
            instance_id: args.instance.map(|i| i.to_vec()).unwrap_or_default(),
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    if args.common.json {
        println!(
            "{}",
            format::render_content_json(&args.content_ref, &blob.payload)
        );
    } else if let Some(path) = &args.out {
        std::fs::write(path, &blob.payload)
            .map_err(|e| CliError::Io(format!("--out {}: {e}", path.display())))?;
    } else {
        // Raw bytes, no trailing newline — binary-safe + pipe-correct.
        std::io::stdout()
            .write_all(&blob.payload)
            .map_err(|e| CliError::Io(e.to_string()))?;
    }
    Ok(())
}

async fn execute_put(args: PutArgs) -> Result<(), CliError> {
    let payload = std::fs::read(&args.file)
        .map_err(|e| CliError::Io(format!("read {}: {e}", args.file.display())))?;
    // Default the advisory display name to the file's basename.
    let filename = args.filename.clone().unwrap_or_else(|| {
        args.file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .put_content(resolved.request(proto::PutContentRequest {
            payload,
            media_type: args.media_type.clone().unwrap_or_default(),
            filename,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    println!("{}", format::render_put_content(&resp, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ContentArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    fn get(parts: &[&str]) -> GetArgs {
        match p(parts).unwrap() {
            ContentArgs::Get(a) => a,
            other @ ContentArgs::Put(_) => panic!("expected get, got {other:?}"),
        }
    }

    fn put(parts: &[&str]) -> PutArgs {
        match p(parts).unwrap() {
            ContentArgs::Put(a) => a,
            other @ ContentArgs::Get(_) => panic!("expected put, got {other:?}"),
        }
    }

    #[test]
    fn legacy_flag_form_still_parses_as_get() {
        let a = get(&[
            "--ref",
            &"cd".repeat(32),
            "--instance",
            &"ab".repeat(16),
            "--out",
            "/tmp/x",
        ]);
        assert_eq!(a.content_ref, [0xcd; 32]);
        assert_eq!(a.instance, Some([0xab; 16]));
        assert_eq!(a.out.as_deref(), Some(std::path::Path::new("/tmp/x")));
    }

    #[test]
    fn explicit_get_without_instance_is_the_uploads_scope() {
        let a = get(&["get", "--ref", &"cd".repeat(32)]);
        assert_eq!(a.content_ref, [0xcd; 32]);
        assert_eq!(a.instance, None, "no --instance ⇒ uploads scope");
    }

    #[test]
    fn get_requires_a_ref() {
        assert!(p(&["get"]).is_err());
        assert!(p(&["--instance", &"ab".repeat(16)]).is_err(), "no --ref");
    }

    #[test]
    fn bad_hex_lengths_are_usage() {
        assert!(p(&["--ref", &"cd".repeat(16), "--instance", &"ab".repeat(16)]).is_err());
        assert!(p(&["--ref", &"cd".repeat(32), "--instance", &"ab".repeat(32)]).is_err());
    }

    #[test]
    fn put_parses_file_and_advisory_fields() {
        let a = put(&[
            "put",
            "/tmp/cat.png",
            "--media-type",
            "image/png",
            "--filename",
            "cat.png",
        ]);
        assert_eq!(a.file, PathBuf::from("/tmp/cat.png"));
        assert_eq!(a.media_type.as_deref(), Some("image/png"));
        assert_eq!(a.filename.as_deref(), Some("cat.png"));
    }

    #[test]
    fn put_requires_exactly_one_file() {
        assert!(p(&["put"]).is_err(), "no file");
        assert!(p(&["put", "/a", "/b"]).is_err(), "two files");
        assert!(p(&["put", "/a", "--bogus"]).is_err(), "unknown flag");
    }

    #[test]
    fn unknown_subcommand_is_usage() {
        assert!(p(&["frobnicate"]).is_err());
        assert!(p(&[]).is_err(), "no args at all");
    }
}
