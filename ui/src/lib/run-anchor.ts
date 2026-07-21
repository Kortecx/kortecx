/**
 * THE RUN ANCHOR — the single server-derived Mote id a run view scopes itself by, and
 * the `/workflows/$instanceId` search that carries it across a navigation.
 *
 * WHY THIS MODULE EXISTS. A `kx serve` is ONE journal with ONE `instance_id` shared by
 * every Invoke, chat turn, scaffold and cron fire, so `/workflows/<instanceId>` alone
 * cannot mean "this run" — the fold returns the whole workspace. Scoping needs a Mote id
 * that belongs to THIS submission (`useProjection`'s `scopeMoteId` → `connectedComponent`),
 * and the gateway returns two of them on `RunHandle`. They are NOT interchangeable:
 *
 *   - `react_chain_salt` is a ReAct CHAIN KEY. A client that sees it non-empty takes the
 *     react settle path (`ListReactTurns` with it as `step_salt`), so the server emits it
 *     ONLY for exactly one tool-granted agentic model step — empty for most Apps, every
 *     pure pipeline, and every multi-agent DAG.
 *   - `terminal_mote_id` is the RUN ANCHOR proper: the bound recipe's sink Mote, populated
 *     for EVERY shape. Empty only from a server older than the field.
 *
 * So the anchor is `reactChainSalt || terminalMoteId` — prefer the salt when present (it
 * is the key the react surfaces already thread through, and it pins the agentic chain
 * exactly), else the terminal Mote (which pins every other shape). That precedence is
 * written down ONCE, here: repeating the `||` at each of the dozen navigation sites is how
 * PR #362 ended up with three scoped routes and eight unscoped ones.
 *
 * Pure and total. An empty result means "this run cannot be scoped" — a caller must SAY
 * so (the run view's unscoped notice), never quietly present the journal as the run.
 */

/**
 * The per-run anchors a started run carries. Structural on purpose: `StartedRun`
 * (`use-invoke`), `SubmittedWorkflow` (`use-submit-workflow`), `RunAppResult`
 * (`use-apps`) and the persisted `RunRecord` (`lib/recent-runs`) all satisfy it, so one
 * helper serves the live-submit sites and the reopened-from-history sites alike.
 */
export interface RunAnchors {
  /** `RunHandle.react_chain_salt` (hex) — the agentic chain key; "" / absent otherwise. */
  readonly reactChainSalt?: string | null;
  /** `RunHandle.terminal_mote_id` (hex) — the run's sink Mote; "" only from an old server. */
  readonly terminalMoteId?: string | null;
}

/** The Mote id a run view scopes by, or `""` when the server gave us neither. */
export function runAnchor(anchors: RunAnchors): string {
  return anchors.reactChainSalt || anchors.terminalMoteId || "";
}

/** The subset of the run route's search this module owns (see `router/routes/workflow-detail`). */
export interface RunViewSearch {
  /** The poll-stop signal — the recipe's terminal (sink) Mote. */
  terminal?: string;
  /** The scope anchor — narrows the fold to this submission's connected component. */
  chain?: string;
}

/**
 * The search to hand a navigation to `/workflows/$instanceId`.
 *
 * Both keys are omitted rather than sent empty: the route's `validateSearch` drops
 * anything that is not 64 hex chars anyway, and an absent `chain` is what makes the view
 * render its honest "showing every step in this journal" notice. Fabricating an anchor to
 * silence that notice would restore the exact bug the scope exists to fix.
 */
export function runViewSearch(anchors: RunAnchors): RunViewSearch {
  const out: RunViewSearch = {};
  if (anchors.terminalMoteId) {
    out.terminal = anchors.terminalMoteId;
  }
  const anchor = runAnchor(anchors);
  if (anchor) {
    out.chain = anchor;
  }
  return out;
}

/**
 * The run-view search for a navigation whose only handle on the run is an ARBITRARY
 * member Mote — a live-feed row's event Mote, an inspector's selected Mote.
 *
 * The scope is a connected-component walk, so any Mote of the submission anchors it just
 * as well as the sink does; what these sites lack is the terminal id, so they carry no
 * poll-stop hint and the view falls back to its frontier-stability heuristic. Kept beside
 * {@link runViewSearch} so the `chain` key name lives in exactly one module.
 */
export function memberMoteSearch(moteId: string | null | undefined): RunViewSearch {
  return moteId ? { chain: moteId } : {};
}

/**
 * The same search as a raw `?…` string, for the one navigation that cannot be a
 * `<Link>` — the run drawer's open-in-new-window `<a href>`.
 */
export function runViewHref(instanceId: string, anchors: RunAnchors): string {
  const search = runViewSearch(anchors);
  const qs = new URLSearchParams(
    Object.entries(search).filter((e): e is [string, string] => e[1] !== undefined),
  ).toString();
  return `/workflows/${instanceId}${qs ? `?${qs}` : ""}`;
}
