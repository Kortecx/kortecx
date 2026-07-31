// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx teams list | members | grants` — the READ-ONLY membership and sharing
//! viewers, at parity with the console.
//!
//! ## Two separate truths, deliberately not merged
//!
//! `members` answers who is in a team. `grants` answers who can do what to an
//! asset. They come from different ledgers and neither implies the other: being
//! in a team is not a grant, and holding a grant does not put you in a team.
//! Presenting them as one table would invite exactly that inference.
//!
//! ## Read-only is the whole surface, and that is a decision
//!
//! There is no `teams add` / `teams grant` here because there is no such RPC.
//! Managing membership and delegating grants across parties is a multi-tenant
//! identity concern; single-node OSS seeds one team from the serve's own auth
//! tokens. A verb that pretended otherwise would be a stub that fails at the
//! wire, which is worse than an honest absence.
//!
//! ## Every field is a display projection
//!
//! Nothing on this surface is identity or a warrant body. `members --asset` will
//! render a member's RESOLVED warrant as pre-formatted scope strings — the
//! ceilings and scopes, never the warrant itself, and never a secret.

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `teams` subcommand.
#[derive(Debug)]
pub enum TeamsSub {
    /// List the teams visible to the caller.
    List,
    /// List one team's effective members.
    Members {
        /// The team's group PartyId.
        team_id: String,
        /// When set, resolve each member's warrant against this asset.
        asset_ref: Option<String>,
    },
    /// List every grant fact on one asset.
    Grants {
        /// `namespace/collection/name` AssetPath handle.
        asset_ref: String,
    },
}

/// Parsed `teams` arguments.
#[derive(Debug)]
pub struct TeamsArgs {
    /// The subcommand.
    pub sub: TeamsSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `teams` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<TeamsArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("teams requires a subcommand: list | members | grants".into())
    })?;

    let mut common = ClientCommon::default();
    let mut positional: Vec<String> = Vec::new();
    let mut asset: Option<String> = None;

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--asset" => asset = Some(next_value(&mut args, "--asset")?),
            other if !other.starts_with("--") => positional.push(other.to_string()),
            other => return Err(CliError::Usage(format!("unknown flag {other}"))),
        }
    }

    let sub = match kw.as_str() {
        "list" => TeamsSub::List,
        "members" => {
            if positional.is_empty() {
                return Err(CliError::Usage(
                    "teams members requires a <team_id> (see `teams list`)".into(),
                ));
            }
            TeamsSub::Members {
                team_id: positional.remove(0),
                asset_ref: asset,
            }
        }
        "grants" => {
            // Accept the asset as a positional OR as --asset: `grants` takes
            // exactly one thing, and making the caller remember which spelling
            // is a papercut with no upside.
            let asset_ref = if positional.is_empty() {
                asset.ok_or_else(|| {
                    CliError::Usage(
                        "teams grants requires an <asset_ref> (namespace/collection/name)".into(),
                    )
                })?
            } else {
                positional.remove(0)
            };
            TeamsSub::Grants { asset_ref }
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown teams subcommand {other:?}: expected list | members | grants"
            )))
        }
    };

    Ok(TeamsArgs { sub, common })
}

/// Run the parsed `teams` subcommand.
pub async fn execute(args: TeamsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        TeamsSub::List => {
            let resp = client
                .list_teams(resolved.request(proto::ListTeamsRequest {})?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_teams_list(&resp, json));
        }
        TeamsSub::Members { team_id, asset_ref } => {
            let resp = client
                .list_team_members(
                    resolved.request(proto::ListTeamMembersRequest { team_id, asset_ref })?,
                )
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_team_members(&resp, json));
        }
        TeamsSub::Grants { asset_ref } => {
            let resp = client
                .list_asset_grants(resolved.request(proto::ListAssetGrantsRequest { asset_ref })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_asset_grants(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse, TeamsSub};

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn members_takes_a_team_id_and_an_optional_asset() {
        let a = parse(v(&["members", "team-a", "--asset", "ns/coll/app"]).into_iter()).unwrap();
        match a.sub {
            TeamsSub::Members { team_id, asset_ref } => {
                assert_eq!(team_id, "team-a");
                assert_eq!(asset_ref.as_deref(), Some("ns/coll/app"));
            }
            other => panic!("expected Members, got {other:?}"),
        }
        // Without --asset the warrant projection is simply absent, not empty.
        let a = parse(v(&["members", "team-a"]).into_iter()).unwrap();
        match a.sub {
            TeamsSub::Members { asset_ref, .. } => assert!(asset_ref.is_none()),
            other => panic!("expected Members, got {other:?}"),
        }
    }

    /// `grants` accepts either spelling of its one argument.
    #[test]
    fn grants_accepts_positional_or_flag() {
        for args in [
            v(&["grants", "ns/coll/app"]),
            v(&["grants", "--asset", "ns/coll/app"]),
        ] {
            match parse(args.into_iter()).unwrap().sub {
                TeamsSub::Grants { asset_ref } => assert_eq!(asset_ref, "ns/coll/app"),
                other => panic!("expected Grants, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_missing_team_id_names_where_to_find_one() {
        let err = parse(v(&["members"]).into_iter()).unwrap_err();
        assert!(err.to_string().contains("teams list"));
    }

    #[test]
    fn an_unknown_subcommand_names_the_alternatives() {
        let err = parse(v(&["add"]).into_iter()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("add"));
        assert!(msg.contains("list | members | grants"));
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = parse(v(&["list", "--json"]).into_iter()).unwrap();
        assert!(a.common.json);
        assert!(matches!(a.sub, TeamsSub::List));
    }
}
