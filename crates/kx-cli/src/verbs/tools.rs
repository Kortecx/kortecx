//! `kx tools list | score` — the advisory toolscout RPCs over the gateway
//! (`ListToolManifests` + `ScoreTaskBundle`). Tri-surface parity with the UI +
//! SDK (W1.A5). Everything here is ADVISORY/DISPLAY-ONLY (SN-8): the scores
//! rank a picker and the verdict is a server-side dry-run of the real lowering
//! gate — neither ever authorizes a tool. The CLI never sends a warrant; the
//! exact `(name, version)` grant gate stays the broker's.

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `tools` subcommand.
#[derive(Debug)]
pub enum ToolsSub {
    /// List the gateway's registered tool manifests (advisory discovery).
    List,
    /// Score a TaskBundle preview: rank every manifest + a lowering dry-run.
    Score(ScoreSpec),
}

/// A `tools score` request, assembled from the flags.
#[derive(Debug)]
pub struct ScoreSpec {
    /// The task instruction (server-validated; non-empty).
    pub intent: String,
    /// Advisory BCP-47-ish language tags (repeatable `--language-tag`).
    pub language_tags: Vec<String>,
    /// The ordered tool sequence as `(tool_id, tool_version)` (repeatable `--tool id@ver`).
    pub tools: Vec<(String, String)>,
    /// The advisory ranking cut in basis points (0..=10000; default 0 = no cut).
    pub tolerance_threshold_bp: u32,
}

/// Parsed `tools` arguments.
#[derive(Debug)]
pub struct ToolsArgs {
    /// The subcommand.
    pub sub: ToolsSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Split a `tool_id@tool_version` token into its parts. The `@` separator
/// matches the convention used everywhere else (e.g. `mcp-echo@1`); a missing
/// `@` is a usage error (a tool is always pinned to a version).
fn parse_tool_ref(raw: &str) -> Result<(String, String), CliError> {
    match raw.rsplit_once('@') {
        Some((id, ver)) if !id.is_empty() && !ver.is_empty() => {
            Ok((id.to_string(), ver.to_string()))
        }
        _ => Err(CliError::Usage(format!(
            "--tool expects <tool_id>@<tool_version> (e.g. fs-read@1), got {raw:?}"
        ))),
    }
}

/// Parse `tools` args (the verb already consumed). The first token selects the
/// subcommand (`list` / `score`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ToolsArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("tools requires a subcommand: list | score".into()))?;

    let mut intent: Option<String> = None;
    let mut language_tags: Vec<String> = Vec::new();
    let mut tools: Vec<(String, String)> = Vec::new();
    let mut tolerance_threshold_bp: u32 = 0;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--intent" => intent = Some(next_value(&mut args, "--intent")?),
            "--tool" => tools.push(parse_tool_ref(&next_value(&mut args, "--tool")?)?),
            "--language-tag" => language_tags.push(next_value(&mut args, "--language-tag")?),
            "--tolerance-threshold-bp" => {
                let raw = next_value(&mut args, "--tolerance-threshold-bp")?;
                tolerance_threshold_bp = raw.parse().map_err(|_| {
                    CliError::Usage(format!(
                        "--tolerance-threshold-bp expects an integer 0..=10000, got {raw:?}"
                    ))
                })?;
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let sub = match kw.as_str() {
        "list" => ToolsSub::List,
        "score" => {
            let intent = intent
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CliError::Usage("tools score requires --intent <text>".into()))?;
            if tools.is_empty() {
                return Err(CliError::Usage(
                    "tools score requires at least one --tool <tool_id>@<tool_version>".into(),
                ));
            }
            ToolsSub::Score(ScoreSpec {
                intent,
                language_tags,
                tools,
                tolerance_threshold_bp,
            })
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown tools subcommand {other:?} (expected list | score)"
            )))
        }
    };
    Ok(ToolsArgs { sub, common })
}

/// Execute `tools`.
pub async fn execute(args: ToolsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        ToolsSub::List => {
            let resp = client
                .list_tool_manifests(resolved.request(proto::ListToolManifestsRequest {})?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_tools_list(&resp, json));
        }
        ToolsSub::Score(spec) => {
            let req = proto::ScoreTaskBundleRequest {
                intent: spec.intent,
                language_tags: spec.language_tags,
                tool_sequence: spec
                    .tools
                    .into_iter()
                    .map(|(tool_id, tool_version)| proto::BundleToolSpec {
                        tool_id,
                        tool_version,
                        description: String::new(),
                        keywords: Vec::new(),
                    })
                    .collect(),
                tolerance_threshold_bp: spec.tolerance_threshold_bp,
            };
            let resp = client
                .score_task_bundle(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_tools_score(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ToolsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_list_and_score() {
        assert!(matches!(p(&["list"]).unwrap().sub, ToolsSub::List));
        let args = p(&[
            "score",
            "--intent",
            "read a file from disk",
            "--tool",
            "fs-read@1",
            "--tool",
            "text-summarize@1",
            "--language-tag",
            "en",
            "--tolerance-threshold-bp",
            "6000",
        ])
        .unwrap();
        let ToolsSub::Score(spec) = args.sub else {
            panic!("expected Score");
        };
        assert_eq!(spec.intent, "read a file from disk");
        assert_eq!(
            spec.tools,
            vec![
                ("fs-read".to_string(), "1".to_string()),
                ("text-summarize".to_string(), "1".to_string())
            ]
        );
        assert_eq!(spec.language_tags, vec!["en"]);
        assert_eq!(spec.tolerance_threshold_bp, 6000);
    }

    #[test]
    fn tool_ref_must_be_id_at_version() {
        assert_eq!(
            parse_tool_ref("fs-read@1").unwrap(),
            ("fs-read".into(), "1".into())
        );
        // A version can itself contain no constraint beyond non-empty; rsplit
        // keeps a name that contains no '@'.
        assert!(parse_tool_ref("fs-read").is_err(), "missing @version");
        assert!(parse_tool_ref("@1").is_err(), "empty id");
        assert!(parse_tool_ref("fs-read@").is_err(), "empty version");
    }

    #[test]
    fn missing_required_and_unknown_are_usage() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["score"]).is_err(), "score needs --intent + --tool");
        assert!(
            p(&["score", "--intent", "x"]).is_err(),
            "score needs at least one --tool"
        );
        assert!(
            p(&["score", "--tool", "fs-read@1"]).is_err(),
            "score needs --intent"
        );
        assert!(
            p(&["score", "--intent", "x", "--tool", "bad-ref"]).is_err(),
            "tool ref needs @version"
        );
        assert!(
            p(&[
                "score",
                "--intent",
                "x",
                "--tool",
                "fs-read@1",
                "--tolerance-threshold-bp",
                "huge"
            ])
            .is_err(),
            "threshold must be an integer"
        );
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
        assert!(p(&["list", "--nope"]).is_err(), "unknown flag");
    }
}
