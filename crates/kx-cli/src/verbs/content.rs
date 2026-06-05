//! `kx content --ref <hex32> --instance <hex16> [--out <file>]` — fetch a
//! committed result. The instance id is the ownership ticket (the run's
//! committed result refs are the authorized set; a non-owned ref is uniformly
//! "not authorized" — no existence oracle). The human path writes RAW bytes
//! (binary-safe); `--out` saves to a file; `--json` emits a hex object.

use std::io::Write;
use std::path::PathBuf;

use kx_proto::proto;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// Parsed `content` arguments.
#[derive(Debug)]
pub struct ContentArgs {
    /// The content ref to fetch (32B).
    pub content_ref: [u8; 32],
    /// The owning run (16B instance id).
    pub instance: [u8; 16],
    /// Write the bytes to this file instead of stdout.
    pub out: Option<PathBuf>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `content` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ContentArgs, CliError> {
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
        content_ref.ok_or_else(|| CliError::Usage("content requires --ref <hex32>".into()))?;
    let instance =
        instance.ok_or_else(|| CliError::Usage("content requires --instance <hex16>".into()))?;
    Ok(ContentArgs {
        content_ref,
        instance,
        out,
        common,
    })
}

/// Execute `content`.
pub async fn execute(args: ContentArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    let blob = client
        .get_content(resolved.request(proto::GetContentRequest {
            content_ref: args.content_ref.to_vec(),
            instance_id: args.instance.to_vec(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ContentArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_ref_instance_and_out() {
        let a = p(&[
            "--ref",
            &"cd".repeat(32),
            "--instance",
            &"ab".repeat(16),
            "--out",
            "/tmp/x",
        ])
        .unwrap();
        assert_eq!(a.content_ref, [0xcd; 32]);
        assert_eq!(a.instance, [0xab; 16]);
        assert_eq!(a.out.as_deref(), Some(std::path::Path::new("/tmp/x")));
    }

    #[test]
    fn missing_required_is_usage() {
        assert!(p(&["--instance", &"ab".repeat(16)]).is_err(), "no --ref");
        assert!(p(&["--ref", &"cd".repeat(32)]).is_err(), "no --instance");
    }

    #[test]
    fn bad_hex_lengths_are_usage() {
        assert!(p(&["--ref", &"cd".repeat(16), "--instance", &"ab".repeat(16)]).is_err());
        assert!(p(&["--ref", &"cd".repeat(32), "--instance", &"ab".repeat(32)]).is_err());
    }
}
