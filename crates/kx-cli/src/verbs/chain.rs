//! `kx chain run "<dsl>" --tasks <tasks.json> [--wait] ...` — author a Tier-1 DAG
//! from the kortecx Chains **string-DSL** and run it via the `SubmitWorkflow` path.
//! The DSL composes task *handles* with three operators — `>` (sequential, a DATA
//! edge parent→child), `&` and `|` (parallel merge, no edge; `&` binds tighter than
//! `|`, matching Python's `>>` / `&` / `|`) — plus `[ ]` grouping. It lowers to the
//! SAME `(steps, edges)` the visual `blueprint run` builds, then feeds the one
//! canonical proto assembly (`blueprint::to_request`). The server
//! still compiles + warrants every step (SN-8) — a chain only changes what is
//! PROPOSED.
//!
//! The cross-surface contract (grammar + canonical lowering + the golden corpus the
//! tests pin against) lives at `tests/golden/chains/SPEC.md` + `corpus.json`; the
//! Python / TypeScript SDKs lower an identical chain to byte-identical topology.
//!
//! ```text
//! kx chain run "a > [b & c]" --tasks tasks.json --wait
//! ```
//! `--tasks` is a JSON object map `{ "a": {"kind":"pure", ...}, ... }` — each value
//! is a [`StepSpec`](crate::verbs::blueprint) (the same shape as a `blueprint`
//! step). A handle that appears more than once is the SAME node (reuse builds DAGs);
//! tasks defined but unused are ignored (lenient). Palette: `pure` / `model` /
//! `tool` (PR-6b-2 — fire a registered tool; `args` lower to the canonical
//! `kx.tool.args` blob).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::verbs::blueprint::{DagSpec, EdgeSpec, StepSpec};
use crate::{format, verbs, wait};

const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// The author-side task map parsed from `--tasks` (handle → step definition).
#[derive(Debug, Deserialize)]
struct TasksFile(BTreeMap<String, StepSpec>);

/// Parsed `chain` arguments.
#[derive(Debug)]
pub struct ChainArgs {
    /// The chain DSL expression (the positional argument).
    pub dsl: String,
    /// The `--tasks <tasks.json>` handle → step map.
    pub tasks: PathBuf,
    /// The chain seed (`--seed`, default 0; folds into entrypoint identity).
    pub seed: u32,
    /// Run to completion and print the committed result (`--wait`).
    pub wait: bool,
    /// `--wait` timeout in seconds.
    pub timeout_secs: u64,
    /// Write the committed result bytes to this file instead of inlining them.
    pub out: Option<PathBuf>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `chain run "<dsl>" --tasks <p> [--seed N] [--wait] ...` (the verb already
/// consumed `run`). The first non-flag token is the DSL expression.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ChainArgs, CliError> {
    let sub = args
        .next()
        .ok_or_else(|| CliError::Usage("chain expects a subcommand (run)".into()))?;
    if sub != "run" {
        return Err(CliError::Usage(format!(
            "unknown chain subcommand {sub:?} (only `run`)"
        )));
    }
    let mut dsl: Option<String> = None;
    let mut tasks: Option<PathBuf> = None;
    let mut seed: u32 = 0;
    let mut wait = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut out: Option<PathBuf> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--tasks" => tasks = Some(PathBuf::from(next_value(&mut args, "--tasks")?)),
            "--seed" => {
                let v = next_value(&mut args, "--seed")?;
                seed = v.parse().map_err(|_| {
                    CliError::Usage(format!("--seed expects an integer, got {v:?}"))
                })?;
            }
            "--wait" => wait = true,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")));
            }
            // The first bare (non-flag) token is the DSL expression; a second is an
            // error (the expression must be a single quoted argument).
            _ => {
                if dsl.is_some() {
                    return Err(CliError::Usage(format!(
                        "chain run takes a single DSL expression; unexpected {flag:?} \
                         (quote the whole chain, e.g. \"a > [b & c]\")"
                    )));
                }
                dsl = Some(flag);
            }
        }
    }

    let dsl = dsl.ok_or_else(|| {
        CliError::Usage("chain run requires a DSL expression, e.g. \"a > [b & c]\"".into())
    })?;
    let tasks =
        tasks.ok_or_else(|| CliError::Usage("chain run requires --tasks <tasks.json>".into()))?;
    Ok(ChainArgs {
        dsl,
        tasks,
        seed,
        wait,
        timeout_secs,
        out,
        common,
    })
}

/// The deterministic lowering of a parsed chain: the node list in first-appearance
/// order + the deduped, sorted edge set. Topology only — `tasks` still resolve each
/// handle to its step at request-assembly time.
#[derive(Debug, PartialEq, Eq)]
struct Lowered {
    /// Handles in first-appearance order; the position is the node index.
    nodes: Vec<String>,
    /// Edges as `(parent_index, child_index)`, deduped + sorted ascending.
    edges: Vec<(u32, u32)>,
}

/// A sub-expression's interface to its neighbours: the node indices a `>` to its
/// LEFT attaches to (`exits`) and the indices a `>` to its RIGHT attaches to
/// (`entries`). Both are order-preserving and deduped.
#[derive(Debug, Clone)]
struct Fragment {
    entries: Vec<u32>,
    exits: Vec<u32>,
}

/// A recursive-descent parser over the chain DSL token stream. It registers nodes
/// (first-appearance order) and accumulates the edge set as it folds operators, so
/// a single left-to-right pass yields the canonical node order.
struct Parser<'a> {
    /// The remaining input (byte slice; the grammar is ASCII).
    src: &'a [u8],
    /// Cursor into `src`.
    pos: usize,
    /// Handles in first-appearance order; index = position.
    nodes: Vec<String>,
    /// `handle → node index` for reuse detection (a repeated handle is one node).
    index_of: HashMap<String, u32>,
    /// The accumulated `(parent, child)` edge set (deduped at insert).
    edges: Vec<(u32, u32)>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            nodes: Vec::new(),
            index_of: HashMap::new(),
            edges: Vec::new(),
        }
    }

    /// Skip ASCII whitespace (insignificant between tokens).
    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    /// Peek the next significant byte (after whitespace) without consuming.
    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.src.get(self.pos).copied()
    }

    /// Register `handle` (or return its existing index on reuse).
    fn intern(&mut self, handle: &str) -> u32 {
        if let Some(&i) = self.index_of.get(handle) {
            return i;
        }
        // The node count is bounded by the input length, so `as u32` cannot wrap on
        // any realistic chain; clamp defensively rather than risk a silent truncation.
        let i = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(handle.to_string());
        self.index_of.insert(handle.to_string(), i);
        i
    }

    /// Add a DATA edge `(parent, child)`, deduping (the DSL can re-state an edge via
    /// handle reuse, e.g. `a > b | a > b`).
    fn add_edge(&mut self, parent: u32, child: u32) {
        if !self.edges.contains(&(parent, child)) {
            self.edges.push((parent, child));
        }
    }

    /// `chain := orexpr` — the entry production.
    fn parse_chain(&mut self) -> Result<Fragment, CliError> {
        self.parse_or()
    }

    /// `orexpr := andexpr ( "|" andexpr )*` — parallel, loosest.
    fn parse_or(&mut self) -> Result<Fragment, CliError> {
        let mut left = self.parse_and()?;
        while self.peek() == Some(b'|') {
            self.pos += 1;
            let right = self.parse_and()?;
            left = merge(left, &right);
        }
        Ok(left)
    }

    /// `andexpr := seqexpr ( "&" seqexpr )*` — parallel, tighter than `|`.
    fn parse_and(&mut self) -> Result<Fragment, CliError> {
        let mut left = self.parse_seq()?;
        while self.peek() == Some(b'&') {
            self.pos += 1;
            let right = self.parse_seq()?;
            left = merge(left, &right);
        }
        Ok(left)
    }

    /// `seqexpr := atom ( ">" atom )*` — sequential, tightest binary. Every exit of
    /// the left joins every entry of the right with a DATA edge.
    fn parse_seq(&mut self) -> Result<Fragment, CliError> {
        let mut left = self.parse_atom()?;
        while self.peek() == Some(b'>') {
            self.pos += 1;
            let right = self.parse_atom()?;
            for &x in &left.exits {
                for &y in &right.entries {
                    self.add_edge(x, y);
                }
            }
            left = Fragment {
                entries: left.entries,
                exits: right.exits,
            };
        }
        Ok(left)
    }

    /// `atom := handle | "[" chain "]"`.
    fn parse_atom(&mut self) -> Result<Fragment, CliError> {
        match self.peek() {
            Some(b'[') => {
                self.pos += 1;
                // An empty group `[]` is a parse error (no expression to compose).
                if self.peek() == Some(b']') {
                    return Err(CliError::Usage(
                        "empty group `[]` in chain expression".into(),
                    ));
                }
                let frag = self.parse_chain()?;
                if self.peek() != Some(b']') {
                    return Err(CliError::Usage(
                        "unbalanced `[` in chain expression (expected `]`)".into(),
                    ));
                }
                self.pos += 1;
                Ok(frag)
            }
            Some(c) if is_handle_start(c) => {
                let handle = self.take_handle();
                let i = self.intern(&handle);
                Ok(Fragment {
                    entries: vec![i],
                    exits: vec![i],
                })
            }
            Some(c) => Err(CliError::Usage(format!(
                "unexpected character {:?} in chain expression",
                c as char
            ))),
            None => Err(CliError::Usage(
                "unexpected end of chain expression (expected a task handle)".into(),
            )),
        }
    }

    /// Consume a `handle := [A-Za-z_][A-Za-z0-9_-]*` (the caller verified the first
    /// byte is a handle start).
    fn take_handle(&mut self) -> String {
        self.skip_ws();
        let start = self.pos;
        // First byte already validated by the caller's `peek`.
        self.pos += 1;
        while self.pos < self.src.len() && is_handle_continue(self.src[self.pos]) {
            self.pos += 1;
        }
        // The slice is ASCII handle bytes by construction → valid UTF-8.
        String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()
    }
}

/// Parallel merge (`&` / `|`): no edges; concatenate entries + exits with an
/// order-preserving dedup (a handle reused across both sides is one node).
fn merge(left: Fragment, right: &Fragment) -> Fragment {
    Fragment {
        entries: dedup_concat(left.entries, &right.entries),
        exits: dedup_concat(left.exits, &right.exits),
    }
}

/// Append `extra` to `base`, keeping first-seen order and dropping duplicates.
fn dedup_concat(mut base: Vec<u32>, extra: &[u32]) -> Vec<u32> {
    for &v in extra {
        if !base.contains(&v) {
            base.push(v);
        }
    }
    base
}

fn is_handle_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_handle_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

/// Parse + lower a chain DSL expression to its canonical `(nodes, edges)`. Runs the
/// fail-closed validations: empty/empty-group/unbalanced → parse error; a cycle (a
/// Kahn topological check) → cycle error. The unknown-handle check happens later,
/// against the `--tasks` map.
fn lower(dsl: &str) -> Result<Lowered, CliError> {
    let mut parser = Parser::new(dsl);
    // An empty (or whitespace-only) expression is a parse error.
    if parser.peek().is_none() {
        return Err(CliError::Usage("empty chain expression".into()));
    }
    let _root = parser.parse_chain()?;
    // Trailing tokens (e.g. a stray `]` or a second expression) are a parse error.
    if parser.peek().is_some() {
        return Err(CliError::Usage(format!(
            "unexpected trailing input in chain expression at byte {}",
            parser.pos
        )));
    }

    let mut edges = parser.edges;
    edges.sort_unstable();
    let nodes = parser.nodes;
    detect_cycle(nodes.len(), &edges)?;
    Ok(Lowered { nodes, edges })
}

/// Kahn's algorithm — a cycle (including a `a > a` self-loop, which yields the edge
/// `(i, i)`) means some node never reaches in-degree 0. Client-side guard; the
/// server compile is the backstop (SN-8).
fn detect_cycle(node_count: usize, edges: &[(u32, u32)]) -> Result<(), CliError> {
    let mut indeg = vec![0usize; node_count];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    for &(p, c) in edges {
        let (p, c) = (p as usize, c as usize);
        adj[p].push(c);
        indeg[c] += 1;
    }
    let mut queue: Vec<usize> = (0..node_count).filter(|&n| indeg[n] == 0).collect();
    let mut visited = 0usize;
    while let Some(n) = queue.pop() {
        visited += 1;
        for &m in &adj[n] {
            indeg[m] -= 1;
            if indeg[m] == 0 {
                queue.push(m);
            }
        }
    }
    if visited == node_count {
        Ok(())
    } else {
        Err(CliError::Usage(
            "chain expression has a cycle (a > … > a); the DAG must be acyclic".into(),
        ))
    }
}

/// Resolve the lowered topology against the `--tasks` map and build the SAME
/// `DagSpec` the visual `blueprint` authors, so the one canonical proto assembly
/// (`blueprint::to_request`) is reused verbatim.
fn build_request(
    lowered: Lowered,
    mut tasks: BTreeMap<String, StepSpec>,
    seed: u32,
) -> Result<kx_proto::proto::SubmitWorkflowRequest, CliError> {
    let mut steps = Vec::with_capacity(lowered.nodes.len());
    for handle in &lowered.nodes {
        let step = tasks.remove(handle).ok_or_else(|| {
            CliError::Usage(format!("unknown task handle {handle:?} (not in --tasks)"))
        })?;
        steps.push(step);
    }
    let edges = lowered
        .edges
        .into_iter()
        .map(|(parent, child)| EdgeSpec {
            parent,
            child,
            edge: "data".to_string(),
            non_cascade: false,
        })
        .collect();
    let spec = DagSpec {
        seed,
        steps,
        edges,
        // The DSL fixes the mode to frozen (the deterministic canonical lowering).
        execution_mode: Some("frozen".to_string()),
    };
    crate::verbs::blueprint::to_request(spec)
}

/// Execute `chain run`.
pub async fn execute(args: ChainArgs) -> Result<(), CliError> {
    let raw = std::fs::read(&args.tasks)
        .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", args.tasks.display())))?;
    let TasksFile(tasks) = serde_json::from_slice(&raw)
        .map_err(|e| CliError::Usage(format!("invalid --tasks JSON: {e}")))?;

    let lowered = lower(&args.dsl)?;
    let req = build_request(lowered, tasks, args.seed)?;

    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let handle = client
        .submit_workflow(resolved.request(req)?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    if args.wait {
        let outcome = wait::await_any_result(
            &mut client,
            &resolved,
            handle.instance_id,
            Duration::from_secs(args.timeout_secs),
        )
        .await?;
        verbs::finish_wait(&outcome, args.common.json, args.out.as_deref())
    } else {
        println!("{}", format::render_submit(&handle, args.common.json));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_proto::proto;
    use serde::Deserialize;

    fn p(parts: &[&str]) -> Result<ChainArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    // ---- arg parsing (the runs.rs / blueprint.rs precedent) ----

    #[test]
    fn requires_run_subcommand_dsl_and_tasks() {
        assert!(p(&[]).is_err(), "no subcommand is a usage error");
        assert!(p(&["nope"]).is_err(), "unknown subcommand is a usage error");
        assert!(p(&["run"]).is_err(), "run without a DSL is a usage error");
        assert!(
            p(&["run", "a > b"]).is_err(),
            "run without --tasks is a usage error"
        );
    }

    #[test]
    fn parses_run_with_flags() {
        let a = p(&[
            "run",
            "a > [b & c]",
            "--tasks",
            "tasks.json",
            "--seed",
            "7",
            "--wait",
            "--json",
            "--timeout-secs",
            "30",
        ])
        .unwrap();
        assert_eq!(a.dsl, "a > [b & c]");
        assert_eq!(a.tasks, PathBuf::from("tasks.json"));
        assert_eq!(a.seed, 7);
        assert!(a.wait && a.common.json);
        assert_eq!(a.timeout_secs, 30);
    }

    #[test]
    fn rejects_a_second_bare_token() {
        // A two-word (unquoted) chain is a usage error — the expression is one arg.
        assert!(p(&["run", "a", "b", "--tasks", "t.json"]).is_err());
    }

    #[test]
    fn rejects_unknown_flag_and_bad_ints() {
        assert!(p(&["run", "a", "--tasks", "t.json", "--bogus"]).is_err());
        assert!(p(&["run", "a", "--tasks", "t.json", "--seed", "many"]).is_err());
        assert!(p(&["run", "a", "--tasks", "t.json", "--timeout-secs", "soon"]).is_err());
    }

    // ---- the golden-corpus parity gate (the tri-surface contract) ----

    /// One corpus case: a success carries `expect`, an error carries `error`.
    #[derive(Debug, Deserialize)]
    struct Case {
        name: String,
        dsl: String,
        #[serde(default)]
        seed: u32,
        tasks: BTreeMap<String, StepSpec>,
        #[serde(default)]
        expect: Option<Expect>,
        #[serde(default)]
        error: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct Expect {
        steps: Vec<ExpectStep>,
        edges: Vec<ExpectEdge>,
    }

    #[derive(Debug, Deserialize)]
    struct ExpectStep {
        kind: String,
        #[serde(default)]
        model_id: String,
        #[serde(default)]
        prompt: String,
        #[serde(default)]
        tool_contract: BTreeMap<String, String>,
        #[serde(default)]
        params: BTreeMap<String, String>,
    }

    #[derive(Debug, Deserialize)]
    struct ExpectEdge {
        parent: u32,
        child: u32,
        edge: String,
    }

    /// The corpus is sibling-pathed off `CARGO_MANIFEST_DIR` (the crate root).
    const CORPUS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/golden/chains/corpus.json"
    ));

    fn load_corpus() -> Vec<Case> {
        serde_json::from_str(CORPUS).expect("the golden chains corpus parses")
    }

    /// The proto `kind` enum back to its DSL string, for asserting against `expect`.
    fn kind_str(kind: i32) -> &'static str {
        match proto::WorkflowStepKind::try_from(kind) {
            Ok(proto::WorkflowStepKind::Pure) => "pure",
            Ok(proto::WorkflowStepKind::Model) => "model",
            Ok(proto::WorkflowStepKind::Exec) => "exec",
            Ok(proto::WorkflowStepKind::Tool) => "tool",
            _ => "unspecified",
        }
    }

    #[test]
    fn corpus_has_both_success_and_error_cases() {
        let cases = load_corpus();
        assert!(cases.len() >= 20, "corpus is fully populated");
        assert!(cases.iter().any(|c| c.expect.is_some()));
        assert!(cases.iter().any(|c| c.error.is_some()));
    }

    #[test]
    fn corpus_success_cases_lower_to_the_expected_steps_and_edges() {
        for case in load_corpus().into_iter().filter(|c| c.expect.is_some()) {
            let expect = case.expect.expect("filtered to success cases");
            let req = build_request(
                lower(&case.dsl).unwrap_or_else(|e| panic!("[{}] lower failed: {e}", case.name)),
                case.tasks,
                case.seed,
            )
            .unwrap_or_else(|e| panic!("[{}] build_request failed: {e}", case.name));

            // seed + frozen mode are part of the canonical lowering.
            assert_eq!(req.seed, case.seed, "[{}] seed", case.name);
            assert_eq!(
                req.execution_mode,
                proto::WorkflowExecutionMode::Frozen as i32,
                "[{}] mode is frozen",
                case.name
            );

            // Steps, in node (first-appearance) order.
            assert_eq!(
                req.steps.len(),
                expect.steps.len(),
                "[{}] step count",
                case.name
            );
            for (i, (got, want)) in req.steps.iter().zip(&expect.steps).enumerate() {
                assert_eq!(
                    kind_str(got.kind),
                    want.kind,
                    "[{}] step {i} kind",
                    case.name
                );
                assert_eq!(
                    got.model_id, want.model_id,
                    "[{}] step {i} model",
                    case.name
                );
                assert_eq!(got.prompt, want.prompt, "[{}] step {i} prompt", case.name);
                let got_contract: BTreeMap<String, String> = got
                    .tool_contract
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                assert_eq!(
                    got_contract, want.tool_contract,
                    "[{}] step {i} tool_contract",
                    case.name
                );
                let want_params: BTreeMap<String, Vec<u8>> = want
                    .params
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone().into_bytes()))
                    .collect();
                let got_params: BTreeMap<String, Vec<u8>> = got
                    .params
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                assert_eq!(got_params, want_params, "[{}] step {i} params", case.name);
            }

            // Edges: every edge is DATA, deduped + sorted by (parent, child).
            let got_edges: Vec<(u32, u32, i32)> = req
                .edges
                .iter()
                .map(|e| (e.parent, e.child, e.edge_kind))
                .collect();
            let want_edges: Vec<(u32, u32, i32)> = expect
                .edges
                .iter()
                .map(|e| {
                    assert_eq!(e.edge, "data", "[{}] every edge is data", case.name);
                    (e.parent, e.child, proto::EdgeKind::Data as i32)
                })
                .collect();
            assert_eq!(got_edges, want_edges, "[{}] edges", case.name);
        }
    }

    #[test]
    fn corpus_error_cases_yield_the_expected_class() {
        for case in load_corpus().into_iter().filter(|c| c.error.is_some()) {
            let class = case.error.expect("filtered to error cases");
            // Parse/cycle errors surface from `lower`; unknown_handle from the tasks
            // resolution in `build_request`.
            let result = lower(&case.dsl)
                .and_then(|low| build_request(low, case.tasks, case.seed).map(|_| ()));
            let err = result
                .expect_err(&format!("[{}] expected an error", case.name))
                .to_string()
                .to_lowercase();
            match class.as_str() {
                "parse" => assert!(
                    err.contains("empty")
                        || err.contains("unbalanced")
                        || err.contains("trailing")
                        || err.contains("unexpected"),
                    "[{}] expected a parse error, got: {err}",
                    case.name
                ),
                "cycle" => assert!(
                    err.contains("cycle"),
                    "[{}] expected a cycle error, got: {err}",
                    case.name
                ),
                "unknown_handle" => assert!(
                    err.contains("unknown task handle"),
                    "[{}] expected an unknown_handle error, got: {err}",
                    case.name
                ),
                other => panic!("[{}] unknown error class {other:?}", case.name),
            }
        }
    }

    // ---- focused unit coverage of the lowering primitives ----

    #[test]
    fn first_appearance_node_order_with_reuse() {
        // `b` first appears before its reuse; the node list is [a, b, c].
        let low = lower("a > b | a > c").unwrap();
        assert_eq!(low.nodes, vec!["a", "b", "c"]);
        assert_eq!(low.edges, vec![(0, 1), (0, 2)]);
    }

    #[test]
    fn precedence_amp_binds_tighter_than_seq_does_not_apply_but_seq_tighter_than_amp() {
        // `a > b & c` == `(a > b) & c`: one edge a→b, c unconnected.
        let low = lower("a > b & c").unwrap();
        assert_eq!(low.edges, vec![(0, 1)]);
        // `a & b > c` == `a & (b > c)`: one edge b→c.
        let low = lower("a & b > c").unwrap();
        assert_eq!(low.edges, vec![(1, 2)]);
    }

    #[test]
    fn self_loop_and_cycle_are_rejected() {
        assert!(lower("a > a").is_err());
        assert!(lower("a > b | b > a").is_err());
    }

    #[test]
    fn empty_and_empty_group_are_parse_errors() {
        assert!(lower("").is_err());
        assert!(lower("   ").is_err());
        assert!(lower("a > []").is_err());
        assert!(lower("[a").is_err(), "unbalanced bracket");
        assert!(lower("a]").is_err(), "trailing close bracket");
    }
}
