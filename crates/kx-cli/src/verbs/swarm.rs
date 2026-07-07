//! `kx swarm` — run a multi-agent orchestration pattern from the CLI without
//! hand-writing the chain DSL. Composes the participants into the equivalent chain
//! TOPOLOGY (the SAME shape the SDK `swarm()`/`supervisor()`/`consensus()` methods
//! author) and delegates to [`crate::verbs::chain::execute`] — so a swarm only changes
//! *how* the `(steps, edges)` are authored; the server still compiles + warrants every
//! step (SN-8). Client-side composition only, byte-identical to the equivalent
//! `kx chain` expression.
//!
//! ```text
//! kx swarm "Research angle A" "Research angle B" --gather "Synthesize both"
//! kx swarm --pattern supervisor --planner "Plan the work" "Do A" "Do B"
//! kx swarm --pattern consensus --vote majority "Classify: spam?" "Classify: spam?"
//! ```

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::verbs::chain::{self, ChainArgs};

/// The orchestration topology to author.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pattern {
    /// `[a0 & a1 & …] > gather` — N parallel agents → a synthesizer.
    Swarm,
    /// `planner > [a0 & a1 & …] > gather` — a lead plans, workers execute, the lead integrates.
    Supervisor,
    /// `[a0 & a1 & …] > reduce` — N voters → a judge (select best-of-N) or an
    /// exact-equality majority (a PURE sink the server reduces).
    Consensus,
}

/// The consensus reduce mode (`--vote`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Vote {
    Judge,
    Majority,
}

/// `config_subset` key marking a PURE sink as an exact-equality consensus vote (mirrors
/// `kx_mote::CONSENSUS_VOTE_KEY` + the SDK's `_CONSENSUS_VOTE_KEY`).
const CONSENSUS_VOTE_KEY: &str = "kx.consensus.vote";

// The default sink prompts — kept byte-identical to the SDK defaults for a consistent UX.
const DEFAULT_SWARM_GATHER: &str = "You are the lead. Synthesize the parallel agents' \
    results above into one coherent, complete answer. Reconcile disagreements, keep what \
    is well-supported, and drop redundancy.";
const DEFAULT_SUPERVISOR_PLANNER: &str = "You are the supervisor. Break the task into \
    clear, independent subtasks for the team and state each subtask precisely, so each \
    teammate knows exactly what to do.";
const DEFAULT_SUPERVISOR_GATHER: &str = "You are the supervisor. Integrate the team's \
    results above into one complete, coherent answer. Reconcile disagreements, keep what \
    is well-supported, drop redundancy.";
const DEFAULT_CONSENSUS_JUDGE: &str = "You are the judge. Read the candidate answers \
    above and choose the single best one; reply with that answer verbatim, without \
    merging or editing the candidates.";

const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Parsed `swarm` arguments.
#[derive(Debug)]
pub struct SwarmArgs {
    /// The agent prompts (bare positionals) — swarm/supervisor workers or consensus voters.
    participants: Vec<String>,
    /// The topology (`--pattern`, default `swarm`).
    pattern: Pattern,
    /// The supervisor lead prompt (`--planner`); ignored for non-supervisor patterns.
    planner: Option<String>,
    /// The gather / judge prompt (`--gather`); a default is used when absent.
    gather: Option<String>,
    /// The consensus reduce mode (`--vote`, default `judge`); used only for consensus.
    vote: Vote,
    /// The shared task appended to each participant prompt (`--goal`).
    goal: String,
    /// The chain seed (`--seed`).
    seed: u32,
    /// Run to completion and print the result (`--wait`).
    wait: bool,
    /// `--wait` timeout in seconds.
    timeout_secs: u64,
    /// `--dry-run` — lower + validate but do NOT submit (needs no gateway).
    dry_run: bool,
    /// Common client flags.
    common: ClientCommon,
}

/// Parse `swarm` args (the verb token already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<SwarmArgs, CliError> {
    let mut participants: Vec<String> = Vec::new();
    let mut pattern = Pattern::Swarm;
    let mut planner: Option<String> = None;
    let mut gather: Option<String> = None;
    let mut vote = Vote::Judge;
    let mut goal = String::new();
    let mut seed: u32 = 0;
    let mut wait = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut dry_run = false;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--pattern" => {
                let v = next_value(&mut args, "--pattern")?;
                pattern = match v.as_str() {
                    "swarm" => Pattern::Swarm,
                    "supervisor" => Pattern::Supervisor,
                    "consensus" => Pattern::Consensus,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--pattern must be swarm|supervisor|consensus, got {other:?}"
                        )))
                    }
                };
            }
            "--planner" => planner = Some(next_value(&mut args, "--planner")?),
            "--gather" => gather = Some(next_value(&mut args, "--gather")?),
            "--vote" => {
                let v = next_value(&mut args, "--vote")?;
                vote = match v.as_str() {
                    "judge" => Vote::Judge,
                    "majority" => Vote::Majority,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--vote must be judge|majority, got {other:?}"
                        )))
                    }
                };
            }
            "--goal" => goal = next_value(&mut args, "--goal")?,
            "--seed" => {
                let v = next_value(&mut args, "--seed")?;
                seed = v.parse().map_err(|_| {
                    CliError::Usage(format!("--seed expects an integer, got {v:?}"))
                })?;
            }
            "--wait" => wait = true,
            "--dry-run" => dry_run = true,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            other => {
                if other.starts_with("--") {
                    return Err(CliError::Usage(format!("unknown flag {other:?}")));
                }
                participants.push(other.to_string());
            }
        }
    }

    if participants.is_empty() {
        return Err(CliError::Usage(
            "swarm needs at least one participant (a bare agent prompt)".into(),
        ));
    }
    Ok(SwarmArgs {
        participants,
        pattern,
        planner,
        gather,
        vote,
        goal,
        seed,
        wait,
        timeout_secs,
        dry_run,
        common,
    })
}

/// Compose a participant prompt = its text + the shared goal (mirrors the SDK `_join_goal`).
fn join_goal(text: &str, goal: &str) -> String {
    if goal.is_empty() {
        text.to_string()
    } else {
        format!("{text}\n\n{goal}")
    }
}

/// A model StepSpec JSON (`{"kind":"model","prompt":…}`) — the inline-task form
/// `chain::collect_tasks` parses.
fn model_task(prompt: &str) -> String {
    serde_json::json!({ "kind": "model", "prompt": prompt }).to_string()
}

/// Lower the parsed swarm to `(dsl, inline_tasks)` — the SAME topology the SDK authors.
fn lower_to_chain(args: &SwarmArgs) -> (String, Vec<(String, String)>) {
    let mut tasks: Vec<(String, String)> = Vec::new();
    // Participant leaves: handles a0..aN.
    let leaves: Vec<String> = (0..args.participants.len())
        .map(|i| format!("a{i}"))
        .collect();
    for (h, p) in leaves.iter().zip(&args.participants) {
        tasks.push((h.clone(), model_task(&join_goal(p, &args.goal))));
    }
    let fan = format!("[{}]", leaves.join(" & "));

    // The sink (gather / judge / reduce) + the DSL for the chosen pattern.
    let dsl = match args.pattern {
        Pattern::Swarm => {
            tasks.push((
                "sink".into(),
                model_task(args.gather.as_deref().unwrap_or(DEFAULT_SWARM_GATHER)),
            ));
            format!("{fan} > sink")
        }
        Pattern::Supervisor => {
            let plan = args
                .planner
                .as_deref()
                .unwrap_or(DEFAULT_SUPERVISOR_PLANNER);
            tasks.push(("lead".into(), model_task(&join_goal(plan, &args.goal))));
            tasks.push((
                "sink".into(),
                model_task(args.gather.as_deref().unwrap_or(DEFAULT_SUPERVISOR_GATHER)),
            ));
            format!("lead > {fan} > sink")
        }
        Pattern::Consensus => {
            match args.vote {
                Vote::Judge => tasks.push((
                    "sink".into(),
                    model_task(args.gather.as_deref().unwrap_or(DEFAULT_CONSENSUS_JUDGE)),
                )),
                // The exact-equality plurality PURE sink the server reduces (SN-8).
                Vote::Majority => tasks.push((
                    "sink".into(),
                    serde_json::json!({
                        "kind": "pure",
                        "params": { CONSENSUS_VOTE_KEY: "majority" }
                    })
                    .to_string(),
                )),
            }
            format!("{fan} > sink")
        }
    };
    (dsl, tasks)
}

/// Execute `swarm` — lower to the equivalent chain, then delegate to `chain::execute`.
pub async fn execute(args: SwarmArgs) -> Result<(), CliError> {
    let (dsl, inline_tasks) = lower_to_chain(&args);
    chain::execute(ChainArgs {
        dsl,
        tasks: None,
        tasks_json: None,
        inline_tasks,
        seed: args.seed,
        context: Vec::new(),
        wait: args.wait,
        timeout_secs: args.timeout_secs,
        out: None,
        emit_blueprint: None,
        dry_run: args.dry_run,
        common: args.common,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<SwarmArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn swarm_lowers_to_parallel_leaves_then_a_gather() {
        let (dsl, tasks) = lower_to_chain(&p(&["A", "B", "--gather", "Merge"]).unwrap());
        assert_eq!(dsl, "[a0 & a1] > sink");
        // 2 leaves + 1 gather; the gather carries the steer prompt.
        assert_eq!(tasks.len(), 3);
        assert!(tasks[2].1.contains("Merge"));
    }

    #[test]
    fn supervisor_prefixes_a_planner_lead() {
        let (dsl, tasks) = lower_to_chain(
            &p(&["--pattern", "supervisor", "--planner", "Plan", "A", "B"]).unwrap(),
        );
        assert_eq!(dsl, "lead > [a0 & a1] > sink");
        assert_eq!(tasks.len(), 4); // a0, a1, lead, sink
        assert!(tasks.iter().any(|(h, j)| h == "lead" && j.contains("Plan")));
    }

    #[test]
    fn consensus_majority_lowers_to_a_pure_marked_sink() {
        let (dsl, tasks) = lower_to_chain(
            &p(&["--pattern", "consensus", "--vote", "majority", "A", "B"]).unwrap(),
        );
        assert_eq!(dsl, "[a0 & a1] > sink");
        let sink = &tasks.iter().find(|(h, _)| h == "sink").unwrap().1;
        assert!(sink.contains("pure") && sink.contains("kx.consensus.vote"));
    }

    #[test]
    fn consensus_judge_uses_a_model_sink() {
        let (_dsl, tasks) = lower_to_chain(&p(&["--pattern", "consensus", "A", "B"]).unwrap());
        let sink = &tasks.iter().find(|(h, _)| h == "sink").unwrap().1;
        assert!(sink.contains("model") && !sink.contains("kx.consensus.vote"));
    }

    #[test]
    fn goal_is_appended_to_each_participant() {
        let (_dsl, tasks) = lower_to_chain(&p(&["A", "--goal", "the topic"]).unwrap());
        assert!(tasks[0].1.contains("the topic"));
    }

    #[test]
    fn empty_participants_is_an_error() {
        assert!(p(&["--gather", "x"]).is_err());
    }

    #[test]
    fn bad_pattern_and_vote_are_errors() {
        assert!(p(&["--pattern", "debate", "A"]).is_err());
        assert!(p(&["--pattern", "consensus", "--vote", "plurality", "A"]).is_err());
    }
}
