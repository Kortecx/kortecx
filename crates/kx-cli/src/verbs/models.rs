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
    /// Model Control v2: `models pull <tag>` (Ollama) or `models pull --url <u>
    /// --sha256 <h>` (direct GGUF) — download + runtime-register a model.
    Pull {
        /// An Ollama registry tag (mutually exclusive with `url`).
        tag: Option<String>,
        /// A `huggingface.co` `/resolve/` GGUF URL (requires `sha256`).
        url: Option<String>,
        /// The expected SHA-256 (hex) of a `url` download.
        sha256: Option<String>,
    },
    /// Model Control v2: `models use <id>` / `models use --clear` — set/clear the
    /// server's active default model. `None` ⇒ clear (back to the primary).
    Use(Option<String>),
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
        CliError::Usage(
            "models requires a subcommand: list | load <id> | offload <id> | \
             pull <tag | --url U --sha256 H> | use <id | --clear>"
                .into(),
        )
    })?;
    // One unified flag loop: cmd-specific flags (--url/--sha256/--clear) + the common
    // client flags + positionals; the subcommand is assembled from the collected pieces.
    let mut common = ClientCommon::default();
    let mut positionals: Vec<String> = Vec::new();
    let mut url: Option<String> = None;
    let mut sha256: Option<String> = None;
    let mut clear = false;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--url" => {
                url = Some(
                    args.next()
                        .ok_or_else(|| CliError::Usage("--url requires a value".into()))?,
                );
            }
            "--sha256" => {
                sha256 = Some(
                    args.next()
                        .ok_or_else(|| CliError::Usage("--sha256 requires a value".into()))?,
                );
            }
            "--clear" => clear = true,
            _ if common.try_consume(&flag, &mut args)? => {}
            _ if !flag.starts_with("--") => positionals.push(flag),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    let cmd = match kw.as_str() {
        "list" => ModelsCmd::List,
        "load" => ModelsCmd::Load(one_positional(&positionals, "load")?),
        "offload" => ModelsCmd::Offload(one_positional(&positionals, "offload")?),
        "pull" => match (positionals.first(), url) {
            (Some(_), Some(_)) => {
                return Err(CliError::Usage(
                    "models pull takes EITHER a <tag> OR --url, not both".into(),
                ))
            }
            (Some(tag), None) => ModelsCmd::Pull {
                tag: Some(tag.clone()),
                url: None,
                sha256: None,
            },
            (None, Some(u)) => {
                let sha = sha256.ok_or_else(|| {
                    CliError::Usage(
                        "models pull --url requires --sha256 <hex> (the download is verified)"
                            .into(),
                    )
                })?;
                ModelsCmd::Pull {
                    tag: None,
                    url: Some(u),
                    sha256: Some(sha),
                }
            }
            (None, None) => {
                return Err(CliError::Usage(
                    "models pull requires a <tag> or --url <url> --sha256 <hex>".into(),
                ))
            }
        },
        "use" => {
            if clear {
                ModelsCmd::Use(None)
            } else {
                Some(one_positional(&positionals, "use")?)
                    .map(|id| ModelsCmd::Use(Some(id)))
                    .ok_or_else(|| CliError::Usage("models use requires <id> or --clear".into()))?
            }
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown models subcommand {other:?} (expected: list | load <id> | \
                 offload <id> | pull <tag|--url> | use <id|--clear>)"
            )))
        }
    };
    Ok(ModelsArgs { cmd, common })
}

/// Exactly one positional model id (fail-closed on zero / many).
fn one_positional(positionals: &[String], verb: &str) -> Result<String, CliError> {
    match positionals {
        [id] => Ok(id.clone()),
        [] => Err(CliError::Usage(format!(
            "models {verb} requires a <model-id>"
        ))),
        _ => Err(CliError::Usage(format!(
            "models {verb} takes exactly one <model-id>"
        ))),
    }
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
        ModelsCmd::Pull { tag, url, sha256 } => {
            let source = if let Some(t) = tag {
                proto::pull_model_request::Source::OllamaTag(t.clone())
            } else {
                proto::pull_model_request::Source::Url(url.clone().unwrap_or_default())
            };
            let resp = client
                .pull_model(resolved.request(proto::PullModelRequest {
                    source: Some(source),
                    sha256: sha256.clone().unwrap_or_default(),
                    model_id: String::new(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            // Deny-by-default refusal (downloads disabled / host not allowlisted): an
            // honest, non-zero-exit error (never a fabricated "started").
            if !resp.accepted {
                return Err(CliError::Usage(format!("pull refused: {}", resp.detail)));
            }
            let model_id = resp.model_id;
            // Poll the background pull to a terminal state. Bounded (a stuck pull
            // cannot hang the CLI forever); each poll re-requests the status.
            for _ in 0..6_000 {
                let st = client
                    .get_pull_status(resolved.request(proto::GetPullStatusRequest {
                        model_id: model_id.clone(),
                    })?)
                    .await
                    .map_err(CliError::from_status)?
                    .into_inner();
                let phase = proto::get_pull_status_response::Phase::try_from(st.phase);
                let terminal = matches!(
                    phase,
                    Ok(proto::get_pull_status_response::Phase::Done
                        | proto::get_pull_status_response::Phase::Failed)
                );
                if terminal {
                    println!(
                        "{}",
                        format::render_pull_status(&model_id, &st, args.common.json)
                    );
                    if matches!(phase, Ok(proto::get_pull_status_response::Phase::Failed)) {
                        return Err(CliError::Usage(format!("pull failed: {}", st.detail)));
                    }
                    return Ok(());
                }
                if !args.common.json {
                    // A single advisory progress line (human only).
                    println!("{}", format::render_pull_status(&model_id, &st, false));
                }
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            }
            return Err(CliError::Usage(
                "pull did not reach a terminal state in time (still running on the server)".into(),
            ));
        }
        ModelsCmd::Use(model_id) => {
            let resp = client
                .set_active_model(resolved.request(proto::SetActiveModelRequest {
                    model_id: model_id.clone().unwrap_or_default(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_set_active(&resp, args.common.json));
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

    #[test]
    fn pull_parses_an_ollama_tag() {
        assert_eq!(
            p(&["pull", "gemma3:12b"]).unwrap().cmd,
            ModelsCmd::Pull {
                tag: Some("gemma3:12b".into()),
                url: None,
                sha256: None,
            }
        );
    }

    #[test]
    fn pull_url_requires_sha256_and_rejects_both_forms() {
        // --url without --sha256 is a usage error (the download is verified).
        assert!(p(&[
            "pull",
            "--url",
            "https://huggingface.co/o/r/resolve/main/m.gguf"
        ])
        .is_err());
        // A valid direct-URL pull.
        assert_eq!(
            p(&[
                "pull",
                "--url",
                "https://huggingface.co/o/r/resolve/main/m.gguf",
                "--sha256",
                "abc123",
            ])
            .unwrap()
            .cmd,
            ModelsCmd::Pull {
                tag: None,
                url: Some("https://huggingface.co/o/r/resolve/main/m.gguf".into()),
                sha256: Some("abc123".into()),
            }
        );
        // A tag AND --url together is refused.
        assert!(p(&["pull", "gemma3:12b", "--url", "https://x/y/resolve/m.gguf"]).is_err());
        // No source at all is refused.
        assert!(p(&["pull"]).is_err());
    }

    #[test]
    fn use_parses_id_and_clear() {
        assert_eq!(
            p(&["use", "kx-serve:gemma"]).unwrap().cmd,
            ModelsCmd::Use(Some("kx-serve:gemma".into()))
        );
        assert_eq!(p(&["use", "--clear"]).unwrap().cmd, ModelsCmd::Use(None));
        // Neither an id nor --clear is a usage error.
        assert!(p(&["use"]).is_err());
    }
}
