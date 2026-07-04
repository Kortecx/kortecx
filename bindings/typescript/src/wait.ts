/**
 * `wait` orchestration — turn a started run into a single result.
 *
 * Composes existing RPCs client-side, exactly like the `kx` CLI (`wait.rs`) and
 * the Python SDK: poll `GetProjection` until the target Mote is terminal, then
 * `GetContent` its committed result. Two strategies share one {@link WaitOutcome}:
 *
 * - **poll** (default, CLI-parity) — re-read the projection every 250 ms.
 * - **events** (opt-in, lower latency) — subscribe to `StreamEvents` and react to
 *   the terminal Mote's delta as it lands, resuming on a CatchupRequired drop.
 */

import { Code, ConnectError } from "@connectrpc/connect";
import type { Client } from "@connectrpc/connect";
import { fromRpcError, rpc } from "./errors.js";
import type { KxGateway } from "./gen/kortecx/v1/gateway_pb.js";
import { MoteSnapshotState } from "./gen/kortecx/v1/gateway_pb.js";

/** The terminal disposition of a waited-on run. */
export const WaitState = {
  Committed: "COMMITTED",
  Failed: "FAILED",
  /** timed out, still in progress, resumable. */
  Running: "RUNNING",
} as const;
export type WaitState = (typeof WaitState)[keyof typeof WaitState];

/** The two wait strategies. */
export type WaitMode = "poll" | "events";

/** Server-derived ids + the terminal disposition (+ result on commit). */
export interface WaitOutcome {
  instanceId: Uint8Array;
  terminalMoteId: Uint8Array;
  state: WaitState;
  resultRef?: Uint8Array;
  payload?: Uint8Array;
}

type Gateway = Client<typeof KxGateway>;

/** Polling cadence — matches the CLI's bounded backoff (never a busy spin). */
const POLL_INTERVAL_MS = 250;

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

function eq(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

function isCommitted(s: MoteSnapshotState): boolean {
  return s === MoteSnapshotState.COMMITTED;
}

function isPending(s: MoteSnapshotState): boolean {
  return s === MoteSnapshotState.PENDING || s === MoteSnapshotState.SCHEDULED;
}

async function fetchContent(
  gw: Gateway,
  instance: Uint8Array,
  ref: Uint8Array,
): Promise<Uint8Array> {
  const blob = await rpc(gw.getContent({ contentRef: ref, instanceId: instance }));
  return blob.payload;
}

async function committedOutcome(
  gw: Gateway,
  instance: Uint8Array,
  mote: Uint8Array,
  ref: Uint8Array | undefined,
): Promise<WaitOutcome> {
  const hasRef = ref !== undefined && ref.length > 0;
  const payload = hasRef ? await fetchContent(gw, instance, ref) : undefined;
  return {
    instanceId: instance,
    terminalMoteId: mote,
    state: WaitState.Committed,
    resultRef: hasRef ? ref : undefined,
    payload,
  };
}

/** Poll until `terminal` is terminal (the `invoke` path). */
export async function pollResult(
  gw: Gateway,
  instance: Uint8Array,
  terminal: Uint8Array,
  timeoutMs: number,
): Promise<WaitOutcome> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const view = await rpc(gw.getProjection({ instanceId: instance }));
    const m = view.motes.find((x) => eq(x.moteId, terminal));
    if (m !== undefined) {
      if (isCommitted(m.state)) {
        return committedOutcome(gw, instance, terminal, m.resultRef);
      }
      if (!isPending(m.state)) {
        return { instanceId: instance, terminalMoteId: terminal, state: WaitState.Failed };
      }
    }
    if (Date.now() >= deadline) {
      return { instanceId: instance, terminalMoteId: terminal, state: WaitState.Running };
    }
    await sleep(POLL_INTERVAL_MS);
  }
}

/** The branches a ReAct turn settles to (vs the live "pending"/"tool" states). */
const REACT_ANSWER = "answer";
const REACT_DEAD = "dead_lettered";

/**
 * gemma3 connector-tool-fire: under the Ollama non-strict UNION `format` a settled react
 * answer turn commits `{"answer": "…"}` instead of free prose; unwrap it to the plain text
 * a caller expects. Mirrors the Rust `kx_toolcall::extract_answer`, the `kx` CLI, and the
 * Python SDK — a byte-identical NO-OP for prose / llama.cpp answers or any body that is not
 * EXACTLY a single-key `{"answer": <string>}` object (presentation only).
 */
function extractAnswer(payload: Uint8Array | undefined): Uint8Array | undefined {
  if (payload === undefined) return payload;
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(payload).trim();
  } catch {
    return payload;
  }
  if (!text.startsWith("{")) return payload;
  let obj: unknown;
  try {
    obj = JSON.parse(text);
  } catch {
    return payload;
  }
  if (
    typeof obj === "object" &&
    obj !== null &&
    !Array.isArray(obj) &&
    Object.keys(obj).length === 1 &&
    typeof (obj as Record<string, unknown>).answer === "string"
  ) {
    return new TextEncoder().encode((obj as Record<string, string>).answer);
  }
  return payload;
}

/**
 * Wait for a ReAct CHAIN to settle (the `invoke` react path).
 *
 * A react chain has no statically-known terminal Mote: the run-salted turn-0 id
 * the gateway hands back never matches the committed turn id, and the settled
 * Answer turn isn't known until the model emits it. So completion is observed via
 * `ListReactTurns` — done when a turn settles to `answer` (resolve its committed
 * content) or `dead_lettered` (terminal failure). Mirrors the runtime's own
 * "resume with get_projection / events" hint (campaign finding F13).
 */
export async function pollReactResult(
  gw: Gateway,
  instance: Uint8Array,
  terminal: Uint8Array,
  timeoutMs: number,
  chainSalt: Uint8Array = new Uint8Array(0),
): Promise<WaitOutcome> {
  // PR-R1: scope the settle poll to THIS invocation's chain (serve shares one
  // journal/instance_id across every Invoke). An empty salt = instance-only scoping.
  const stepSalt = chainSalt.length > 0 ? chainSalt : undefined;
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const resp = await rpc(gw.listReactTurns({ instanceId: instance, stepSalt }));
    const answer = resp.turns.find((t) => t.branch === REACT_ANSWER);
    if (answer !== undefined) {
      const view = await rpc(gw.getProjection({ instanceId: instance }));
      const m = view.motes.find((x) => eq(x.moteId, answer.turnMoteId));
      const outcome = await committedOutcome(gw, instance, answer.turnMoteId, m?.resultRef);
      return { ...outcome, payload: extractAnswer(outcome.payload) };
    }
    const dead = resp.turns.find((t) => t.branch === REACT_DEAD);
    if (dead !== undefined) {
      return { instanceId: instance, terminalMoteId: dead.turnMoteId, state: WaitState.Failed };
    }
    if (Date.now() >= deadline) {
      return { instanceId: instance, terminalMoteId: terminal, state: WaitState.Running };
    }
    await sleep(POLL_INTERVAL_MS);
  }
}

/** Poll until ANY Mote commits (the `submit` path — no terminal id). */
export async function pollAny(
  gw: Gateway,
  instance: Uint8Array,
  timeoutMs: number,
): Promise<WaitOutcome> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const view = await rpc(gw.getProjection({ instanceId: instance }));
    const committed = view.motes.find((x) => isCommitted(x.state));
    if (committed !== undefined) {
      return committedOutcome(gw, instance, committed.moteId, committed.resultRef);
    }
    const first = view.motes[0];
    if (first !== undefined && view.motes.every((x) => !isPending(x.state))) {
      return { instanceId: instance, terminalMoteId: first.moteId, state: WaitState.Failed };
    }
    if (Date.now() >= deadline) {
      return { instanceId: instance, terminalMoteId: new Uint8Array(0), state: WaitState.Running };
    }
    await sleep(POLL_INTERVAL_MS);
  }
}

/** Wait via the live event stream (lower latency than the poll). */
export async function eventsResult(
  gw: Gateway,
  instance: Uint8Array,
  terminal: Uint8Array,
  timeoutMs: number,
): Promise<WaitOutcome> {
  const deadline = Date.now() + timeoutMs;
  let cursor = 0n;
  for (;;) {
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      return { instanceId: instance, terminalMoteId: terminal, state: WaitState.Running };
    }
    try {
      for await (const frame of gw.streamEvents(
        { instanceId: instance, sinceSeq: cursor },
        { timeoutMs: remaining },
      )) {
        for (const d of frame.deltas) {
          if (d.kind.case === "committed" && eq(d.kind.value.moteId, terminal)) {
            const rr = d.kind.value.resultRef;
            return committedOutcome(gw, instance, terminal, rr.length > 0 ? rr : undefined);
          }
          if (d.kind.case === "failed" && eq(d.kind.value.moteId, terminal)) {
            return { instanceId: instance, terminalMoteId: terminal, state: WaitState.Failed };
          }
        }
        cursor = frame.nextSeq;
      }
    } catch (e) {
      const ce = ConnectError.from(e);
      if (ce.code === Code.ResourceExhausted) continue; // CatchupRequired: resume from cursor
      if (ce.code === Code.DeadlineExceeded || ce.code === Code.Canceled) {
        return { instanceId: instance, terminalMoteId: terminal, state: WaitState.Running };
      }
      throw fromRpcError(e);
    }
    // The snapshot stream ended before the terminal committed — re-subscribe from
    // the cursor after a short backoff (avoids a hot loop; the terminal commits soon).
    await sleep(POLL_INTERVAL_MS);
  }
}
