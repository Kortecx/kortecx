//! `kx signatures list | get --id <hex32> | register --manifest-file <path>` —
//! the catalog task-signature RPCs over the gateway. The manifest is opaque
//! encoded bytes (the CLI never decodes it); the server derives the id (SN-8).

use std::path::PathBuf;

use kx_proto::proto;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `signatures` subcommand.
#[derive(Debug)]
pub enum SignaturesSub {
    /// List registered signatures.
    List,
    /// Fetch one signature's manifest by id (32B).
    Get([u8; 32]),
    /// Register a signature from an encoded-manifest file.
    Register(PathBuf),
}

/// Parsed `signatures` arguments.
#[derive(Debug)]
pub struct SignaturesArgs {
    /// The subcommand.
    pub sub: SignaturesSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `signatures` args (the verb already consumed). The first token selects
/// the subcommand (`list` / `get` / `register`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<SignaturesArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("signatures requires a subcommand: list | get | register".into())
    })?;

    let mut id: Option<[u8; 32]> = None;
    let mut manifest_file: Option<PathBuf> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--id" => id = Some(take_fixed::<_, 32>(&mut args, "--id")?),
            "--manifest-file" => {
                manifest_file = Some(PathBuf::from(next_value(&mut args, "--manifest-file")?));
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let sub = match kw.as_str() {
        "list" => SignaturesSub::List,
        "get" => SignaturesSub::Get(
            id.ok_or_else(|| CliError::Usage("signatures get requires --id <hex32>".into()))?,
        ),
        "register" => SignaturesSub::Register(manifest_file.ok_or_else(|| {
            CliError::Usage("signatures register requires --manifest-file <path>".into())
        })?),
        other => {
            return Err(CliError::Usage(format!(
                "unknown signatures subcommand {other:?} (expected list | get | register)"
            )))
        }
    };
    Ok(SignaturesArgs { sub, common })
}

/// Execute `signatures`.
pub async fn execute(args: SignaturesArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        SignaturesSub::List => {
            let resp = client
                .list_signatures(resolved.request(proto::ListSignaturesRequest {})?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_signatures_list(&resp, json));
        }
        SignaturesSub::Get(id) => {
            let resp = client
                .get_signature(resolved.request(proto::GetSignatureRequest {
                    signature_id: id.to_vec(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_signature_get(&resp, json));
        }
        SignaturesSub::Register(path) => {
            let manifest = std::fs::read(&path)
                .map_err(|e| CliError::Io(format!("--manifest-file {}: {e}", path.display())))?;
            let resp = client
                .register_signature(resolved.request(proto::RegisterSignatureRequest { manifest })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_signature_register(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<SignaturesArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_each_subcommand() {
        assert!(matches!(p(&["list"]).unwrap().sub, SignaturesSub::List));
        assert!(matches!(
            p(&["get", "--id", &"ab".repeat(32)]).unwrap().sub,
            SignaturesSub::Get(id) if id == [0xab; 32]
        ));
        assert!(matches!(
            p(&["register", "--manifest-file", "/tmp/m"]).unwrap().sub,
            SignaturesSub::Register(_)
        ));
    }

    #[test]
    fn missing_required_and_unknown_are_usage() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["get"]).is_err(), "get needs --id");
        assert!(p(&["get", "--id", "abcd"]).is_err(), "id wrong length");
        assert!(p(&["register"]).is_err(), "register needs --manifest-file");
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
        assert!(p(&["list", "--nope"]).is_err(), "unknown flag");
    }
}
