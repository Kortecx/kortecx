/**
 * The embeddable agent-runner — `runAgent` (PR-9c-1). The headline adoption entry
 * (GR18/D149): give a goal (+ optional context + inputs), the runtime completes it
 * AGENTICALLY — reasoning, calling permission-gated tools, and returning a reasoned
 * answer PLUS the AUDITED set of actions it took.
 *
 * A thin wrapper over `invoke("kx/recipes/react")` — NEVER `submitRun` (BLOCKER #5);
 * the warrant is always SERVER-DERIVED (SN-8), the client only parameterizes the
 * published recipe. NODE entry (uses the zero-config default client); the `web` /
 * `chains` entrypoints are explicit-client and do NOT import this module.
 *
 * `inputs` fold into the goal prompt — the `kx/recipes/react` contract has no
 * structured input slot today (instruction / max_turns / max_tool_calls only).
 */

import { AgentResult, assembleActions } from "./agent-result.js";
import { type ImageInput, REACT_RECIPE_HANDLE } from "./client.js";
import { defaultClient } from "./defaults.js";
import type { KxClient } from "./node.js";
import type { Result, Run } from "./run.js";

/** The recipe's anchored bounded-loop budget (mirrors Agent + the UI's planReactArgs).
 *  T-MULTI-ELEMENT-TOOLCALLS: the tool-call cap rose 6 → 20 (decoupled from max_turns —
 *  a turn can fire N tools); overridable per call via `maxToolCalls`. */
const DEFAULT_MAX_TURNS = 8;
const DEFAULT_MAX_TOOL_CALLS = 20;

export interface RunAgentOptions {
  /** What to accomplish — becomes the react recipe's instruction. */
  goal: string;
  /** Published context-bundle handles (PR-7) the server resolves + injects. */
  context?: readonly string[];
  /** Structured inputs folded into the goal prompt (no structured recipe slot yet). */
  inputs?: Readonly<Record<string, string>>;
  /** Max total tool calls (default 20, ceiling 20; a turn may fire several at once). */
  maxToolCalls?: number;
  /** AGENTIC-VISION: an image to ground the agentic run. When set, binds
   * `kx/recipes/react-vision` (form-gated) so the served VLM reasons over the image on
   * EVERY turn; fail-closed (a usage error) when no vision model is served. */
  image?: ImageInput;
  /** `true` (default) returns an {@link AgentResult}; `false` returns the started {@link Run}. */
  wait?: boolean;
  timeoutMs?: number;
  /** An explicit client; defaults to the zero-config process-wide default client. */
  client?: KxClient;
}

function foldInputs(goal: string, inputs?: Readonly<Record<string, string>>): string {
  const lines = inputs ? Object.entries(inputs).map(([k, v]) => `- ${k}: ${v}`) : [];
  return lines.length === 0 ? goal : `${goal}\n\nInputs:\n${lines.join("\n")}`;
}

/**
 * Complete `opts.goal` agentically and return an {@link AgentResult} (the final
 * answer + the audited tool actions). With `wait: false` returns the started
 * {@link Run} (assemble the result later via `listReactTurns`). Throws
 * `KxRunFailed` if the chain dead-letters and `KxWaitTimeout` on a timeout — same
 * as `invoke({ wait: true })`.
 */
export async function runAgent(opts: RunAgentOptions): Promise<AgentResult | Run> {
  const kx = opts.client ?? defaultClient();
  const baseArgs = {
    instruction: foldInputs(opts.goal, opts.inputs),
    max_turns: DEFAULT_MAX_TURNS,
    max_tool_calls: opts.maxToolCalls ?? DEFAULT_MAX_TOOL_CALLS,
  };
  // AGENTIC-VISION: an attached image binds the image-grounded react recipe (form-gated)
  // so the served VLM reasons over it on every turn; fail-closed when no vision model.
  const { handle, args } = opts.image
    ? await kx.bindReactVision(baseArgs, opts.image)
    : { handle: REACT_RECIPE_HANDLE, args: baseArgs };
  if (opts.wait === false) {
    return (await kx.invoke(handle, args, {
      context: opts.context,
      wait: false,
    })) as Run;
  }
  const result = (await kx.invoke(handle, args, {
    context: opts.context,
    wait: true,
    timeoutMs: opts.timeoutMs,
  })) as Result;
  // PR-R1: scope the action fetch to THIS invocation's chain (serve's shared journal).
  const page = await kx.listReactTurns({
    instanceId: result.instanceId,
    stepSalt: result.reactChainSalt || undefined,
  });
  return new AgentResult(
    result.text,
    result.payload,
    assembleActions(page.turns),
    result.instanceId,
    result.instanceId,
  );
}
