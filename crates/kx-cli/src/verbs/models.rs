//! `kx models list | load <id> | offload <id>` — model discovery + local
//! lifecycle over the gateway (`ListModels` / `LoadModel` / `OffloadModel`).
//! Cross-surface parity with the UI + the SDKs. `list` is DISPLAY-ONLY (SN-8) and
//! reports live RAM residency (`loaded`); `load`/`offload` warm/evict a REGISTERED
//! model (an unregistered id is a fail-closed `not found`). An FFI-free serve
//! answers `list` with an honest empty list and refuses lifecycle (`unimplemented`).

use kx_proto::proto;

use crate::client::ClientCommon;
use crate::error::CliError;
use crate::format;

/// The `models` subcommand.
#[derive(Debug, PartialEq, Eq)]
pub enum ModelsCmd {
    /// `models list` — discover models + live residency.
    List,
    /// `models load <id>` — warm a registered model into RAM.
    Load(String),
    /// `models offload <id>` — evict a registered model from RAM.
    Offload(String),
}

/// Parsed `models` arguments.
#[derive(Debug)]
pub struct ModelsArgs {
    /// The selected subcommand.
    pub cmd: ModelsCmd,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `models` args (the verb already consumed). The first token selects the
/// subcommand; `load`/`offload` take a model-id positional.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ModelsArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("models requires a subcommand: list | load <id> | offload <id>".into())
    })?;
    let cmd = match kw.as_str() {
        "list" => ModelsCmd::List,
        "load" | "offload" => {
            let id = args
                .next()
                .ok_or_else(|| CliError::Usage(format!("models {kw} requires a <model-id>")))?;
            if kw == "load" {
                ModelsCmd::Load(id)
            } else {
                ModelsCmd::Offload(id)
            }
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown models subcommand {other:?} (expected: list | load <id> | offload <id>)"
            )))
        }
    };
    let mut common = ClientCommon::default();
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        return Err(CliError::Usage(format!("unknown flag {flag:?}")));
    }
    Ok(ModelsArgs { cmd, common })
}

/// Execute `models list | load | offload`.
pub async fn execute(args: ModelsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    match &args.cmd {
        ModelsCmd::List => {
            let resp = client
                .list_models(resolved.request(proto::ListModelsRequest {})?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_models(&resp, args.common.json));
        }
        ModelsCmd::Load(model_id) => {
            let resp = client
                .load_model(resolved.request(proto::LoadModelRequest {
                    model_id: model_id.clone(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_load_model(&resp, args.common.json));
        }
        ModelsCmd::Offload(model_id) => {
            let resp = client
                .offload_model(resolved.request(proto::OffloadModelRequest {
                    model_id: model_id.clone(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_offload_model(&resp, args.common.json));
        }
    }
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
        assert_eq!(p(&["list"]).unwrap().cmd, ModelsCmd::List);
        let a = p(&["list", "--json"]).unwrap();
        assert!(a.common.json);
    }

    #[test]
    fn load_and_offload_parse_the_model_id() {
        assert_eq!(
            p(&["load", "kx-serve:gemma"]).unwrap().cmd,
            ModelsCmd::Load("kx-serve:gemma".into())
        );
        assert_eq!(
            p(&["offload", "kx-serve:qwen", "--json"]).unwrap().cmd,
            ModelsCmd::Offload("kx-serve:qwen".into())
        );
    }

    #[test]
    fn missing_id_or_unknown_subcommand_is_usage() {
        assert!(p(&[]).is_err());
        assert!(p(&["score"]).is_err());
        assert!(p(&["load"]).is_err()); // missing <id>
        assert!(p(&["offload"]).is_err());
        assert!(p(&["list", "--bogus"]).is_err());
    }
}
