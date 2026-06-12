//! `kx models list` — model discovery over the gateway (`ListModels`, Batch A).
//! Tri-surface parity with the UI picker + the SDKs. DISPLAY-ONLY (SN-8): model
//! *selection* stays a recipe ENUM free-param validated server-side at binding
//! — listing a model never routes one. An FFI-free serve answers an honest
//! empty list.

use kx_proto::proto;

use crate::client::ClientCommon;
use crate::error::CliError;
use crate::format;

/// Parsed `models` arguments.
#[derive(Debug)]
pub struct ModelsArgs {
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `models` args (the verb already consumed). The first token selects the
/// subcommand (only `list` today).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ModelsArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("models requires a subcommand: list".into()))?;
    if kw != "list" {
        return Err(CliError::Usage(format!(
            "unknown models subcommand {kw:?} (expected: list)"
        )));
    }
    let mut common = ClientCommon::default();
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        return Err(CliError::Usage(format!("unknown flag {flag:?}")));
    }
    Ok(ModelsArgs { common })
}

/// Execute `models list`.
pub async fn execute(args: ModelsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_models(resolved.request(proto::ListModelsRequest {})?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    println!("{}", format::render_models(&resp, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ModelsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn list_parses_with_and_without_json() {
        assert!(p(&["list"]).is_ok());
        let a = p(&["list", "--json"]).unwrap();
        assert!(a.common.json);
    }

    #[test]
    fn missing_or_unknown_subcommand_is_usage() {
        assert!(p(&[]).is_err());
        assert!(p(&["score"]).is_err());
        assert!(p(&["list", "--bogus"]).is_err());
    }
}
