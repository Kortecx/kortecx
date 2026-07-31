// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx policy put | list | delete | assign` — the durable Policy/Role registry's
//! operator surface, at parity with the console and the SDKs.
//!
//! ## A role NARROWS; it never grants
//!
//! `--tool` names a tool the role narrows TO. It does not hand the party that
//! tool: the effective authority is the INTERSECTION of every present leg, so
//! assigning a role can only ever take capability away. Naming a tool the party
//! could not fire anyway simply drops out of the intersection.
//!
//! That is what makes this surface safe to expose at all. Under the obvious
//! alternative — "a role GRANTS the tools it names" — anyone who could write a
//! role could write themselves a capability. Under intersection, the worst a
//! malicious role can do is refuse work.
//!
//! ## An empty role and no role are different things
//!
//! `kx policy put ops` with no `--tool` creates a role that narrows to NOTHING,
//! and a party assigned it can fire nothing. A party with NO role assigned is
//! un-narrowed and resolves exactly as it did before a registry existed. The
//! empty list is a decision; the absent assignment is the default.
//!
//! ## Deleting a role WIDENS
//!
//! Removing a role that parties are still assigned to returns them to their
//! un-narrowed authority. That is the honest outcome — refusing the delete would
//! make a role permanent the moment it was used — but it is a widening, so it is
//! worth saying out loud rather than discovering.

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;

/// The `policy` subcommand.
#[derive(Debug)]
pub enum PolicySub {
    /// Create or update a role.
    Put {
        /// Role name (the catalog key).
        name: String,
        /// Free-form description; never parsed for enforcement.
        description: String,
        /// `(tool_id, tool_version)` pairs the role narrows to.
        tools: Vec<(String, String)>,
    },
    /// List the caller's roles.
    List {
        /// Page size; 0 = server default (100, clamped to 256).
        limit: u32,
    },
    /// Delete a role by exact name.
    Delete {
        /// Role name.
        name: String,
    },
    /// Assign a role to a party, or unassign with `--none`.
    Assign {
        /// The PartyId the role applies to.
        party: String,
        /// The role name; empty ⇒ unassign.
        name: String,
    },
}

/// Parsed `policy` arguments.
#[derive(Debug)]
pub struct PolicyArgs {
    /// The subcommand.
    pub sub: PolicySub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `policy` args (the verb already consumed).
#[allow(clippy::too_many_lines)]
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<PolicyArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("policy requires a subcommand: put | list | delete | assign".into())
    })?;

    let mut common = ClientCommon::default();
    let mut positional: Vec<String> = Vec::new();
    let mut description = String::new();
    let mut tools: Vec<(String, String)> = Vec::new();
    let mut limit: u32 = 0;
    let mut unassign = false;

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--description" => description = next_value(&mut args, &flag)?,
            "--tool" => {
                let raw = next_value(&mut args, &flag)?;
                // `id@version`, the same spelling `kx scripts` and the grant-set
                // key already use. A missing version is refused rather than
                // defaulted: "@1" guessed wrong is a role that silently narrows
                // to nothing.
                let (id, version) = raw.split_once('@').ok_or_else(|| {
                    CliError::Usage(format!(
                        "--tool expects `tool_id@tool_version`, got {raw:?}"
                    ))
                })?;
                if id.is_empty() || version.is_empty() {
                    return Err(CliError::Usage(format!(
                        "--tool needs both halves of `tool_id@tool_version`, got {raw:?}"
                    )));
                }
                tools.push((id.to_string(), version.to_string()));
            }
            "--limit" => {
                limit = next_value(&mut args, &flag)?
                    .parse()
                    .map_err(|_| CliError::Usage("--limit expects a number".into()))?;
            }
            "--none" => unassign = true,
            other if !other.starts_with("--") => positional.push(other.to_string()),
            other => return Err(CliError::Usage(format!("unknown flag {other}"))),
        }
    }

    let need = |p: &[String], n: usize, verb: &str, what: &str| -> Result<(), CliError> {
        if p.len() < n {
            return Err(CliError::Usage(format!("policy {verb} expects {what}")));
        }
        Ok(())
    };

    let sub = match kw.as_str() {
        "put" => {
            need(
                &positional,
                1,
                "put",
                "<name> [--description D] [--tool id@ver ...]",
            )?;
            PolicySub::Put {
                name: positional.remove(0),
                description,
                tools,
            }
        }
        "list" => PolicySub::List { limit },
        "delete" => {
            need(&positional, 1, "delete", "<name>")?;
            PolicySub::Delete {
                name: positional.remove(0),
            }
        }
        "assign" => {
            if unassign {
                need(&positional, 1, "assign", "<party> --none")?;
                PolicySub::Assign {
                    party: positional.remove(0),
                    name: String::new(),
                }
            } else {
                need(
                    &positional,
                    2,
                    "assign",
                    "<party> <role>  (or <party> --none)",
                )?;
                let party = positional.remove(0);
                PolicySub::Assign {
                    party,
                    name: positional.remove(0),
                }
            }
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown policy subcommand {other:?}: expected put | list | delete | assign"
            )))
        }
    };

    Ok(PolicyArgs { sub, common })
}

/// Run the parsed `policy` subcommand.
#[allow(clippy::too_many_lines)] // a flat per-subcommand dispatcher (the verbs' convention)
pub async fn execute(args: PolicyArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        PolicySub::Put {
            name,
            description,
            tools,
        } => {
            let tool_count = tools.len();
            let req = proto::PutPolicyRoleRequest {
                name: name.clone(),
                description,
                tools: tools
                    .into_iter()
                    .map(|(tool_id, tool_version)| proto::PolicyRoleTool {
                        tool_id,
                        tool_version,
                    })
                    .collect(),
            };
            let resp = client
                .put_policy_role(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": name,
                        "created": resp.created,
                        "tools": tool_count,
                    })
                );
            } else {
                let verb = if resp.created { "created" } else { "updated" };
                // Say what an empty role MEANS. "0 tools" reads like a no-op; it
                // is the opposite.
                if tool_count == 0 {
                    println!("{verb} role {name}  (narrows to NOTHING — an assigned party can fire no tool)");
                } else {
                    println!("{verb} role {name}  (narrows to {tool_count} tool(s))");
                }
            }
            Ok(())
        }

        PolicySub::List { limit } => {
            let req = proto::ListPolicyRolesRequest { limit };
            let resp = client
                .list_policy_roles(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "roles": resp.roles.iter().map(|r| serde_json::json!({
                            "name": r.name,
                            "description": r.description,
                            "tools": r.tools.iter()
                                .map(|t| format!("{}@{}", t.tool_id, t.tool_version))
                                .collect::<Vec<_>>(),
                            "created_unix_ms": r.created_unix_ms,
                            "updated_unix_ms": r.updated_unix_ms,
                        })).collect::<Vec<_>>(),
                    })
                );
            } else if resp.roles.is_empty() {
                println!("no policy roles");
            } else {
                for r in &resp.roles {
                    let tools = if r.tools.is_empty() {
                        "(narrows to nothing)".to_string()
                    } else {
                        r.tools
                            .iter()
                            .map(|t| format!("{}@{}", t.tool_id, t.tool_version))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    println!("{}  {}", r.name, tools);
                    if !r.description.is_empty() {
                        println!("    {}", r.description);
                    }
                }
            }
            Ok(())
        }

        PolicySub::Delete { name } => {
            let req = proto::DeletePolicyRoleRequest { name: name.clone() };
            let resp = client
                .delete_policy_role(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "name": name, "removed": resp.removed })
                );
            } else if resp.removed {
                // Deleting WIDENS anyone still assigned. Say so at the moment it
                // happens, not in a doc nobody re-reads.
                println!("deleted role {name}  (any party still assigned it is now UN-NARROWED)");
            } else {
                println!("role {name:?}: not found");
            }
            Ok(())
        }

        PolicySub::Assign { party, name } => {
            let unassigning = name.is_empty();
            let req = proto::AssignPolicyRoleRequest {
                party: party.clone(),
                name: name.clone(),
            };
            let resp = client
                .assign_policy_role(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "party": party,
                        "role": name,
                        "assigned": resp.assigned,
                    })
                );
            } else if unassigning {
                println!("unassigned {party}  (back to un-narrowed authority)");
            } else {
                println!("assigned {name} to {party}");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, PolicySub};

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn put_parses_a_name_description_and_tools() {
        let a = parse(
            v(&[
                "put",
                "ops",
                "--description",
                "read-only ops",
                "--tool",
                "fs.read@1",
                "--tool",
                "http.get@2",
            ])
            .into_iter(),
        )
        .unwrap();
        match a.sub {
            PolicySub::Put {
                name,
                description,
                tools,
            } => {
                assert_eq!(name, "ops");
                assert_eq!(description, "read-only ops");
                assert_eq!(
                    tools,
                    vec![
                        ("fs.read".to_string(), "1".to_string()),
                        ("http.get".to_string(), "2".to_string()),
                    ]
                );
            }
            other => panic!("expected Put, got {other:?}"),
        }
    }

    /// An empty role is LEGAL and is not the same as no role.
    #[test]
    fn put_accepts_a_role_that_narrows_to_nothing() {
        let a = parse(v(&["put", "locked-down"]).into_iter()).unwrap();
        match a.sub {
            PolicySub::Put { name, tools, .. } => {
                assert_eq!(name, "locked-down");
                assert!(tools.is_empty());
            }
            other => panic!("expected Put, got {other:?}"),
        }
    }

    /// A half-blank `--tool` is refused, not defaulted. A guessed version is a
    /// role that silently narrows to nothing.
    #[test]
    fn a_tool_without_a_version_is_refused() {
        let err = parse(v(&["put", "ops", "--tool", "fs.read"]).into_iter()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("tool_id@tool_version"),
            "the refusal must name the expected spelling, got {msg:?}"
        );
        let err = parse(v(&["put", "ops", "--tool", "fs.read@"]).into_iter()).unwrap_err();
        assert!(err.to_string().contains("both halves"));
    }

    #[test]
    fn assign_takes_a_party_and_a_role() {
        let a = parse(v(&["assign", "party-a", "ops"]).into_iter()).unwrap();
        match a.sub {
            PolicySub::Assign { party, name } => {
                assert_eq!(party, "party-a");
                assert_eq!(name, "ops");
            }
            other => panic!("expected Assign, got {other:?}"),
        }
    }

    /// `--none` is the documented unassign, and it takes only the party.
    #[test]
    fn assign_none_unassigns() {
        let a = parse(v(&["assign", "party-a", "--none"]).into_iter()).unwrap();
        match a.sub {
            PolicySub::Assign { party, name } => {
                assert_eq!(party, "party-a");
                assert!(name.is_empty(), "an empty name IS the unassign");
            }
            other => panic!("expected Assign, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_subcommand_names_the_alternatives() {
        let err = parse(v(&[]).into_iter()).unwrap_err();
        let msg = err.to_string();
        for expected in ["put", "list", "delete", "assign"] {
            assert!(msg.contains(expected), "{msg:?} should name {expected}");
        }
    }

    #[test]
    fn an_unknown_subcommand_names_the_alternatives() {
        let err = parse(v(&["frobnicate"]).into_iter()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("frobnicate"));
        assert!(msg.contains("put | list | delete | assign"));
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = parse(v(&["list", "--json", "--limit", "5"]).into_iter()).unwrap();
        assert!(a.common.json);
        match a.sub {
            PolicySub::List { limit } => assert_eq!(limit, 5),
            other => panic!("expected List, got {other:?}"),
        }
    }
}
