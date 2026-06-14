//! `kx recipe list | search` — the recipe catalog + advisory discovery over the
//! gateway (`ListRecipes` + `SearchRecipes`, PR-4 Batch D). Tri-surface parity
//! with the UI + SDK. Everything here is ADVISORY/DISPLAY-ONLY (SN-8): the
//! `score_bp` ranks a picker, never authorizes a recipe. The CLI never sends a
//! warrant; `kx invoke` stays the authorization gate.

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `recipe` subcommand.
#[derive(Debug)]
pub enum RecipeSub {
    /// List the gateway's provisioned, invocable recipe handles (+ advisory metadata).
    List,
    /// Rank the recipes against an intent (+ optional keywords); advisory discovery.
    Search(SearchSpec),
}

/// A `recipe search` request, assembled from the flags.
#[derive(Debug)]
pub struct SearchSpec {
    /// The free-text task intent (server-validated).
    pub intent: String,
    /// Optional keyword filters (repeatable `--keyword`).
    pub keywords: Vec<String>,
    /// Optional result cap (server-clamped); `None` ⇒ the server default.
    pub limit: Option<u32>,
}

/// Parsed `recipe` arguments.
#[derive(Debug)]
pub struct RecipeArgs {
    /// The subcommand.
    pub sub: RecipeSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `recipe` args (the verb already consumed). The first token selects the
/// subcommand (`list` / `search`); `search` takes its intent as the next
/// positional (or via `--intent`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<RecipeArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("recipe requires a subcommand: list | search".into()))?;

    let mut intent: Option<String> = None;
    let mut keywords: Vec<String> = Vec::new();
    let mut limit: Option<u32> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--intent" => intent = Some(next_value(&mut args, "--intent")?),
            "--keyword" => keywords.push(next_value(&mut args, "--keyword")?),
            "--limit" => {
                let raw = next_value(&mut args, "--limit")?;
                limit = Some(raw.parse().map_err(|_| {
                    CliError::Usage(format!("--limit expects a positive integer, got {raw:?}"))
                })?);
            }
            // A bare positional after `search` is the intent (the common case:
            // `kx recipe search "agent loop"`).
            other if !other.starts_with('-') && intent.is_none() => {
                intent = Some(other.to_string());
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let sub = match kw.as_str() {
        "list" => RecipeSub::List,
        "search" => {
            let intent = intent.filter(|s| !s.is_empty()).ok_or_else(|| {
                CliError::Usage("recipe search requires an intent (positional or --intent)".into())
            })?;
            RecipeSub::Search(SearchSpec {
                intent,
                keywords,
                limit,
            })
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown recipe subcommand {other:?} (expected list | search)"
            )))
        }
    };
    Ok(RecipeArgs { sub, common })
}

/// Execute `recipe`.
pub async fn execute(args: RecipeArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        RecipeSub::List => {
            let resp = client
                .list_recipes(resolved.request(proto::ListRecipesRequest {})?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_recipes_list(&resp, json));
        }
        RecipeSub::Search(spec) => {
            let req = proto::SearchRecipesRequest {
                intent: spec.intent,
                keywords: spec.keywords,
                limit: spec.limit,
            };
            let resp = client
                .search_recipes(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_recipes_search(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<RecipeArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_list_and_search() {
        assert!(matches!(p(&["list"]).unwrap().sub, RecipeSub::List));
        let args = p(&["search", "agent loop", "--keyword", "tools", "--limit", "5"]).unwrap();
        let RecipeSub::Search(spec) = args.sub else {
            panic!("expected Search");
        };
        assert_eq!(spec.intent, "agent loop");
        assert_eq!(spec.keywords, vec!["tools"]);
        assert_eq!(spec.limit, Some(5));
    }

    #[test]
    fn search_accepts_intent_flag_or_positional() {
        let a = p(&["search", "--intent", "chat"]).unwrap();
        let RecipeSub::Search(s) = a.sub else {
            panic!("expected Search");
        };
        assert_eq!(s.intent, "chat");
        assert_eq!(s.limit, None);
    }

    #[test]
    fn missing_required_and_unknown_are_usage() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["search"]).is_err(), "search needs an intent");
        assert!(
            p(&["search", "x", "--limit", "huge"]).is_err(),
            "limit must be an integer"
        );
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
        assert!(p(&["list", "--nope"]).is_err(), "unknown flag");
    }
}
