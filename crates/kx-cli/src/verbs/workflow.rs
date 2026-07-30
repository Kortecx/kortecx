// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx workflow save | list | get | run | delete | propose` — the durable Workflow
//! entity's operator surface, at parity with the console and both SDKs.
//!
//! ## `propose` stops on purpose
//!
//! `propose` renders the plan the planner produced (or the refusal) and **exits**.
//! It never calls `SaveWorkflow`. That separation IS the safety property: a
//! natural-language goal produces a PREVIEW a human reads, and saving it is a
//! second, explicit invocation. Collapsing the two into one convenient command
//! would turn "describe what you want" into "the machine authored and installed
//! something", which is exactly the boundary OSS does not cross.
//!
//! ## `run` never submits a client warrant
//!
//! `run` drives `RunWorkflow`, whose warrants are server-derived from the stored
//! definition. It is never `SubmitRun` — the CLI has no path that hands the server
//! a warrant the caller wrote (BLOCKER #5).
//!
//! ## Drafts
//!
//! `--draft` saves with `lifecycle = "draft"`. Finishing a draft is the SAME save
//! with the flag dropped: identical bytes under a changed lifecycle is a real
//! write, and the server reports `deduplicated=false` for it. There is no separate
//! "finish" verb, because there is no separate mechanism.

use std::path::PathBuf;
use std::time::Duration;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::{format, hex, verbs, wait};

/// Default `--wait` budget, matching the App run verb.
const DEFAULT_WAIT_SECS: u64 = 120;

/// The `workflow` subcommand.
#[derive(Debug)]
pub enum WorkflowSub {
    /// Save (upsert) a workflow envelope from a JSON file.
    Save {
        /// Catalog handle (`namespace/collection/name`).
        handle: String,
        /// Path to the `kortecx.workflow/v1` envelope JSON.
        file: PathBuf,
        /// Save as a draft (`lifecycle = "draft"`).
        draft: bool,
    },
    /// List saved workflows.
    List {
        /// Page size; 0 = server default.
        limit: u32,
        /// Exclusive cursor.
        after: String,
    },
    /// Fetch one workflow's stored envelope.
    Get {
        /// Catalog handle.
        handle: String,
        /// Write the envelope bytes here instead of stdout.
        output: Option<PathBuf>,
    },
    /// Run a saved workflow.
    Run {
        /// Catalog handle.
        handle: String,
        /// Entry-arg JSON object.
        args: String,
        /// Per-run HITL posture.
        require_approval: bool,
        /// Block until the run settles.
        wait: bool,
        /// `--wait` budget in seconds.
        timeout_secs: u64,
        /// Write the committed payload here.
        out: Option<PathBuf>,
    },
    /// Delete a saved workflow (cascades branch binding, lock, triggers).
    Delete {
        /// Catalog handle.
        handle: String,
    },
    /// Ask the planner for a plan. PREVIEW ONLY — it saves nothing.
    Propose {
        /// The natural-language goal.
        goal: String,
    },
}

/// Parsed `workflow` arguments.
#[derive(Debug)]
pub struct WorkflowArgs {
    /// The subcommand.
    pub sub: WorkflowSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `workflow` args (the verb already consumed).
#[allow(clippy::too_many_lines)]
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<WorkflowArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage(
            "workflow requires a subcommand: save | list | get | run | delete | propose".into(),
        )
    })?;

    let mut common = ClientCommon::default();
    let mut positional: Vec<String> = Vec::new();
    let mut file: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut goal = String::new();
    let mut args_json = String::new();
    let mut limit: u32 = 0;
    let mut after = String::new();
    let mut draft = false;
    let mut require_approval = false;
    let mut do_wait = false;
    let mut timeout_secs = DEFAULT_WAIT_SECS;

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--file" => file = Some(PathBuf::from(next_value(&mut args, "--file")?)),
            "--output" => output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            "--goal" => goal = next_value(&mut args, "--goal")?,
            "--args" => args_json = next_value(&mut args, "--args")?,
            "--after" => after = next_value(&mut args, "--after")?,
            "--limit" => {
                limit = next_value(&mut args, "--limit")?
                    .parse()
                    .map_err(|_| CliError::Usage("--limit expects a number".into()))?;
            }
            "--timeout" => {
                timeout_secs = next_value(&mut args, "--timeout")?
                    .parse()
                    .map_err(|_| CliError::Usage("--timeout expects seconds".into()))?;
            }
            "--draft" => draft = true,
            "--require-approval" => require_approval = true,
            "--wait" => do_wait = true,
            other if !other.starts_with("--") => positional.push(other.to_string()),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let handle = |p: &[String], verb: &str| -> Result<String, CliError> {
        p.first()
            .filter(|s| !s.is_empty())
            .cloned()
            .ok_or_else(|| CliError::Usage(format!("workflow {verb} requires a <HANDLE>")))
    };

    let sub = match kw.as_str() {
        "save" => {
            // `kx workflow save <HANDLE> <FILE>` or `--file`.
            let h = handle(&positional, "save")?;
            let f = file
                .or_else(|| positional.get(1).map(PathBuf::from))
                .ok_or_else(|| {
                    CliError::Usage(
                        "workflow save requires the envelope JSON (<FILE> or --file)".into(),
                    )
                })?;
            WorkflowSub::Save {
                handle: h,
                file: f,
                draft,
            }
        }
        "list" => WorkflowSub::List { limit, after },
        "get" => WorkflowSub::Get {
            handle: handle(&positional, "get")?,
            output,
        },
        "run" => WorkflowSub::Run {
            handle: handle(&positional, "run")?,
            args: args_json,
            require_approval,
            wait: do_wait,
            timeout_secs,
            out,
        },
        "delete" => WorkflowSub::Delete {
            handle: handle(&positional, "delete")?,
        },
        "propose" => {
            let g = if goal.is_empty() {
                positional.join(" ")
            } else {
                goal
            };
            if g.trim().is_empty() {
                return Err(CliError::Usage(
                    "workflow propose requires a goal (--goal or a positional phrase)".into(),
                ));
            }
            WorkflowSub::Propose { goal: g }
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown workflow subcommand {other:?} \
                 (expected save | list | get | run | delete | propose)"
            )))
        }
    };
    Ok(WorkflowArgs { sub, common })
}

/// Execute `workflow`.
#[allow(clippy::too_many_lines)]
pub async fn execute(args: WorkflowArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        WorkflowSub::Save {
            handle,
            file,
            draft,
        } => {
            let envelope = std::fs::read(&file)
                .map_err(|e| CliError::Io(format!("{}: {e}", file.display())))?;
            let req = proto::SaveWorkflowRequest {
                handle: handle.clone(),
                envelope_json: envelope,
                source_digest: Vec::new(),
                lifecycle: if draft { "draft".into() } else { String::new() },
            };
            let resp = client
                .save_workflow(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "handle": resp.handle,
                        "workflow_ref": hex::encode(&resp.workflow_ref),
                        "deduplicated": resp.deduplicated,
                        "lifecycle": if draft { "draft" } else { "" },
                    })
                );
            } else {
                let state = if draft { " (draft)" } else { "" };
                let dedup = if resp.deduplicated {
                    " — unchanged"
                } else {
                    ""
                };
                println!(
                    "saved {}{state}  ref={}{dedup}",
                    resp.handle,
                    hex::encode(&resp.workflow_ref)
                );
            }
            Ok(())
        }

        WorkflowSub::List { limit, after } => {
            let req = proto::ListWorkflowsRequest {
                limit,
                after_handle: after,
            };
            let resp = client
                .list_workflows(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                let rows: Vec<_> = resp
                    .workflows
                    .iter()
                    .map(|w| {
                        serde_json::json!({
                            "handle": w.handle,
                            "workflow_ref": hex::encode(&w.workflow_ref),
                            "name": w.name,
                            "version": w.version,
                            "description": w.description,
                            "tags": w.tags,
                            "step_count": w.step_count,
                            "delivers": w.delivers,
                            "lifecycle": w.lifecycle,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({ "workflows": rows, "has_more": resp.has_more })
                );
            } else if resp.workflows.is_empty() {
                println!("no workflows");
            } else {
                for w in &resp.workflows {
                    let state = if w.lifecycle == "draft" {
                        "  [draft]"
                    } else {
                        ""
                    };
                    println!(
                        "{}  {} steps  {}{state}",
                        w.handle,
                        w.step_count,
                        if w.delivers.is_empty() {
                            &w.description
                        } else {
                            &w.delivers
                        }
                    );
                }
                if resp.has_more {
                    println!(
                        "… more (use --after {})",
                        resp.workflows.last().map_or("", |w| &w.handle)
                    );
                }
            }
            Ok(())
        }

        WorkflowSub::Get { handle, output } => {
            let req = proto::GetWorkflowRequest {
                handle: handle.clone(),
            };
            let resp = client
                .get_workflow(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if !resp.found {
                // Uniform not-found: absent and not-owned are indistinguishable
                // by design (no existence oracle).
                return Err(CliError::Runtime(format!("workflow {handle:?}: not found")));
            }
            if let Some(path) = output {
                std::fs::write(&path, &resp.envelope_json)
                    .map_err(|e| CliError::Io(format!("--output {}: {e}", path.display())))?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "handle": handle,
                            "written": path.display().to_string(),
                            "bytes": resp.envelope_json.len(),
                            "workflow_digest": hex::encode(&resp.workflow_digest),
                        })
                    );
                } else {
                    println!(
                        "wrote {} ({} bytes)",
                        path.display(),
                        resp.envelope_json.len()
                    );
                }
            } else {
                // The stored bytes ARE the answer; emit them verbatim.
                println!("{}", String::from_utf8_lossy(&resp.envelope_json));
            }
            Ok(())
        }

        WorkflowSub::Run {
            handle,
            args: args_json,
            require_approval,
            wait: do_wait,
            timeout_secs,
            out,
        } => {
            let req = proto::RunWorkflowRequest {
                handle: handle.clone(),
                args: args_json.into_bytes(),
                require_approval,
            };
            let submitted = client
                .run_workflow(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();

            if !do_wait {
                println!("{}", format::render_submit(&submitted, json));
                return Ok(());
            }

            // A stored workflow has a statically-known terminal Mote, so the run
            // anchor is always populated. The first-committed fallback is an
            // OLD-SERVER degrade only: on a shared journal "first committed" is
            // some other submission's result wearing this run's return type.
            let outcome = if submitted.terminal_mote_id.is_empty() {
                wait::await_any_result(
                    &mut client,
                    &resolved,
                    submitted.instance_id,
                    Duration::from_secs(timeout_secs),
                )
                .await?
            } else {
                wait::await_result(
                    &mut client,
                    &resolved,
                    submitted.instance_id,
                    submitted.terminal_mote_id,
                    Duration::from_secs(timeout_secs),
                )
                .await?
            };
            verbs::finish_wait(&outcome, json, out.as_deref())
        }

        WorkflowSub::Delete { handle } => {
            let req = proto::DeleteWorkflowRequest {
                handle: handle.clone(),
            };
            let resp = client
                .delete_workflow(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "handle": handle,
                        "removed": resp.removed,
                        "branch_unbound": resp.branch_unbound,
                        "lock_cleared": resp.lock_cleared,
                        "triggers_removed": resp.triggers_removed,
                    })
                );
            } else if resp.removed {
                println!(
                    "deleted {handle}  (branch_unbound={} lock_cleared={} triggers_removed={})",
                    resp.branch_unbound, resp.lock_cleared, resp.triggers_removed
                );
            } else {
                println!("{handle}: nothing to delete");
            }
            Ok(())
        }

        WorkflowSub::Propose { goal } => {
            let req = proto::ProposeWorkflowRequest { goal };
            let resp = client
                .propose_workflow(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            render_proposal(&resp, json)
        }
    }
}

/// Render a `ProposeWorkflow` result. This function saves NOTHING — see the
/// module doc.
fn render_proposal(resp: &proto::ProposeWorkflowResponse, json: bool) -> Result<(), CliError> {
    use proto::propose_workflow_response::Result as R;
    match resp.result.as_ref() {
        Some(R::Plan(plan)) => {
            if json {
                let steps: Vec<_> = plan
                    .steps
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "role": s.role,
                            "intent": s.intent,
                            "model_id": s.model_id,
                        })
                    })
                    .collect();
                let edges: Vec<_> = plan
                    .edges
                    .iter()
                    .map(|e| serde_json::json!({ "parent": e.parent, "child": e.child }))
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "proposed": true, "steps": steps, "edges": edges,
                        "note": "preview only — nothing was saved",
                    })
                );
            } else {
                println!(
                    "proposed plan ({} steps) — NOTHING SAVED:",
                    plan.steps.len()
                );
                for (i, s) in plan.steps.iter().enumerate() {
                    println!("  [{i}] {}  {}", s.role, s.intent);
                }
                for e in &plan.edges {
                    println!("      {} -> {}", e.parent, e.child);
                }
                println!("\nreview it, then: kx workflow save <handle> <file>");
            }
            Ok(())
        }
        Some(R::Rejected(r)) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "proposed": false, "reason": r.reason })
                );
            } else {
                println!("refused: {}", r.reason);
            }
            // A refusal is a real answer, not a CLI failure.
            Ok(())
        }
        None => Err(CliError::Runtime(
            "ProposeWorkflow returned neither a plan nor a refusal".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, WorkflowSub};

    fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn save_takes_handle_and_file_positionally() {
        let a = parse(args(&["save", "acme/ops/nightly", "wf.json"])).unwrap();
        match a.sub {
            WorkflowSub::Save {
                handle,
                file,
                draft,
            } => {
                assert_eq!(handle, "acme/ops/nightly");
                assert_eq!(file.to_string_lossy(), "wf.json");
                assert!(!draft);
            }
            other => panic!("expected Save, got {other:?}"),
        }
    }

    #[test]
    fn draft_is_a_flag_not_a_separate_verb() {
        let a = parse(args(&["save", "h", "f.json", "--draft"])).unwrap();
        assert!(matches!(a.sub, WorkflowSub::Save { draft: true, .. }));
    }

    #[test]
    fn propose_accepts_a_bare_phrase() {
        let a = parse(args(&["propose", "summarise", "the", "incident"])).unwrap();
        match a.sub {
            WorkflowSub::Propose { goal } => assert_eq!(goal, "summarise the incident"),
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn run_defaults_to_no_wait_and_no_approval() {
        let a = parse(args(&["run", "acme/ops/nightly"])).unwrap();
        match a.sub {
            WorkflowSub::Run {
                wait,
                require_approval,
                ..
            } => {
                assert!(!wait);
                assert!(!require_approval);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_handle_is_a_usage_error() {
        assert!(parse(args(&["get"])).is_err());
        assert!(parse(args(&["delete"])).is_err());
        assert!(parse(args(&["propose"])).is_err());
    }

    #[test]
    fn an_unknown_subcommand_names_the_alternatives() {
        let e = parse(args(&["finish", "h"])).unwrap_err();
        let msg = format!("{e}");
        assert!(msg.contains("save"), "got {msg}");
        // There is deliberately no `finish` verb: finishing a draft is `save`
        // without --draft, because that IS the mechanism.
        assert!(!msg.contains("finish |"), "got {msg}");
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = parse(args(&[
            "list",
            "--endpoint",
            "http://127.0.0.1:1",
            "--json",
        ]))
        .unwrap();
        assert!(a.common.json);
    }
}
