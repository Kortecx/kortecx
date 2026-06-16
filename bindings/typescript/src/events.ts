/**
 * Event-stream consumers.
 *
 * Mirrors the CLI `events` verb: {@link streamDeltas} without `follow` reads one
 * snapshot (`since` → the current journal boundary) and stops; with `follow` it
 * consumes the server's live tail and transparently reconnects from the last
 * cursor on a `CatchupRequired` (`RESOURCE_EXHAUSTED`) drop — no delta lost or
 * duplicated. The WS helpers ({@link wsUrl} / {@link wsDelta}) back the optional
 * browser/firewall-friendly R5 bridge consumed by `node.ts` / `web.ts`.
 */

import { Code, ConnectError } from "@connectrpc/connect";
import type { Client } from "@connectrpc/connect";
import { KxConnectError, fromRpcError } from "./errors.js";
import type { KxGateway } from "./gen/kortecx/v1/gateway_pb.js";
import { TokenChunk } from "./tokens.js";
import { Delta, GlobalDelta } from "./types.js";

type Gateway = Client<typeof KxGateway>;

/** Yield a run's event deltas (one snapshot, or the live tail with `follow`). */
export async function* streamDeltas(
  gw: Gateway,
  instance: Uint8Array,
  since: bigint,
  follow: boolean,
  signal?: AbortSignal,
): AsyncIterable<Delta> {
  let cursor = since;
  for (;;) {
    try {
      const opts = signal ? { signal } : undefined;
      for await (const frame of gw.streamEvents({ instanceId: instance, sinceSeq: cursor }, opts)) {
        for (const d of frame.deltas) {
          const view = Delta.fromProto(d);
          if (view !== null) yield view;
        }
        cursor = frame.nextSeq;
        if (!follow && frame.journalBoundary) return;
      }
    } catch (e) {
      const ce = ConnectError.from(e);
      if (follow && ce.code === Code.ResourceExhausted) continue; // resume from cursor
      throw fromRpcError(e);
    }
    if (!follow) return;
  }
}

/**
 * Yield the operator-global cross-run event tail (Batch C `StreamAllEvents`):
 * one snapshot, or the live tail with `follow`. Same frame/cursor contract as
 * {@link streamDeltas} — transparently reconnects from the last `next_seq` on a
 * `CatchupRequired` (`RESOURCE_EXHAUSTED`) drop, no delta lost or duplicated.
 */
export async function* streamAllDeltas(
  gw: Gateway,
  since: bigint,
  follow: boolean,
  signal?: AbortSignal,
): AsyncIterable<GlobalDelta> {
  let cursor = since;
  for (;;) {
    try {
      const opts = signal ? { signal } : undefined;
      for await (const frame of gw.streamAllEvents({ sinceSeq: cursor }, opts)) {
        for (const d of frame.deltas) {
          yield GlobalDelta.fromProto(d);
        }
        cursor = frame.nextSeq;
        if (!follow && frame.journalBoundary) return;
      }
    } catch (e) {
      const ce = ConnectError.from(e);
      if (follow && ce.code === Code.ResourceExhausted) continue; // resume from cursor
      throw fromRpcError(e);
    }
    if (!follow) return;
  }
}

/**
 * The WS bridge base URL: an explicit ws endpoint, or the gRPC endpoint's
 * scheme/host mapped to the conventional WS port (50152). Mirrors the Python
 * SDK's `_ws_url`.
 */
function wsBase(grpcEndpoint: string, wsEndpoint: string | undefined): string {
  if (wsEndpoint) {
    return wsEndpoint.replace(/\/+$/, "");
  }
  let rest = grpcEndpoint;
  let scheme = "wss";
  if (rest.startsWith("http://")) {
    scheme = "ws";
    rest = rest.slice("http://".length);
  } else if (rest.startsWith("https://")) {
    scheme = "wss";
    rest = rest.slice("https://".length);
  }
  const hostPort = rest.split("/")[0] ?? "";
  const host = hostPort.split(":").slice(0, -1).join(":") || hostPort.split(":")[0] || hostPort;
  return `${scheme}://${host}:50152`;
}

/** Derive the per-run `/v1/events` WS URL (see {@link wsBase}). */
export function wsUrl(
  grpcEndpoint: string,
  wsEndpoint: string | undefined,
  instanceHex: string,
  since: bigint,
): string {
  return `${wsBase(grpcEndpoint, wsEndpoint)}/v1/events?instance=${instanceHex}&since=${since.toString()}`;
}

/** Derive the global `/v1/events/all` WS URL (Batch C — no instance param). */
export function wsAllUrl(
  grpcEndpoint: string,
  wsEndpoint: string | undefined,
  since: bigint,
): string {
  return `${wsBase(grpcEndpoint, wsEndpoint)}/v1/events/all?since=${since.toString()}`;
}

/**
 * Yield a model mote's ADVISORY tokens over the native gRPC stream (PR-4.2 /
 * T-STREAM1): the NEW bytes per decode step, until the terminal `done` chunk.
 * The committed `result_ref` stays the authority — a consumer reconciles to it.
 * An old gateway without this RPC throws (mapped via {@link fromRpcError}).
 */
export async function* streamModelTokens(
  gw: Gateway,
  instance: Uint8Array,
  mote: Uint8Array,
  since: bigint,
  signal?: AbortSignal,
): AsyncIterable<TokenChunk> {
  try {
    const opts = signal ? { signal } : undefined;
    for await (const chunk of gw.streamModelTokens(
      { instanceId: instance, moteId: mote, sinceSeq: since },
      opts,
    )) {
      const view = TokenChunk.fromProto(chunk);
      yield view;
      if (view.done) return;
    }
  } catch (e) {
    throw fromRpcError(e);
  }
}

/** Derive the per-mote `/v1/tokens` WS URL (PR-4.2 — see {@link wsUrl}). */
export function wsTokenUrl(
  grpcEndpoint: string,
  wsEndpoint: string | undefined,
  instanceHex: string,
  moteHex: string,
  since: bigint,
): string {
  return `${wsBase(grpcEndpoint, wsEndpoint)}/v1/tokens?instance=${instanceHex}&mote=${moteHex}&since=${since.toString()}`;
}

/**
 * Parse a stream of WS JSON token-chunk messages into {@link TokenChunk}s. Unlike
 * the event channel (one frame of many deltas per message), each token message is
 * exactly ONE chunk. Stops after the terminal `done` chunk.
 */
export async function* wsTokenChunksFromMessages(
  messages: AsyncIterable<string>,
): AsyncIterable<TokenChunk> {
  for await (const message of messages) {
    let obj: Record<string, unknown>;
    try {
      obj = JSON.parse(message);
    } catch {
      continue;
    }
    const view = TokenChunk.fromWs(obj);
    yield view;
    if (view.done) return;
  }
}

/** Map one R5 WS JSON delta (`type` discriminant, hex ids) to a {@link Delta}. */
/**
 * Map the wire `nd_class` STRING tag back to its `NdClass` discriminant — the
 * inverse of the gateway's `nd_str`. The committed WS delta carries `nd_class`
 * as a string (`"pure"`/`"read_only_nondet"`/`"world_mutating"`/`"unspecified"`),
 * but {@link Delta}/{@link GlobalDelta} model it numerically (matching the gRPC
 * proto). Absent/unknown ⇒ `null` — honest, never a fabricated `0`.
 */
export function ndClassFromTag(tag: string | null): number | null {
  switch (tag) {
    case "pure":
      return 1;
    case "read_only_nondet":
      return 2;
    case "world_mutating":
      return 3;
    case "unspecified":
      return 0;
    default:
      return null;
  }
}

export function wsDelta(obj: Record<string, unknown>): Delta | null {
  const kind = obj.type as string | undefined;
  const seq = Number(obj.seq ?? 0);
  const str = (k: string): string | null =>
    typeof obj[k] === "string" ? (obj[k] as string) : null;
  const num = (k: string): number | null => (obj[k] != null ? Number(obj[k]) : null);
  switch (kind) {
    case "committed":
      return new Delta(
        seq,
        "committed",
        str("mote_id"),
        str("result_ref"),
        ndClassFromTag(str("nd_class")),
      );
    case "failed":
      return new Delta(seq, "failed", str("mote_id"), null, null, num("reason_class"));
    case "repudiated":
      return new Delta(
        seq,
        "repudiated",
        null,
        null,
        null,
        null,
        str("target_mote_id"),
        num("target_committed_seq"),
      );
    case "effect_staged":
      return new Delta(seq, "effect_staged", str("mote_id"));
    default:
      return null;
  }
}

/** Parse a stream of WS JSON frame messages into deltas (shared by node/web). */
export async function* wsDeltasFromMessages(messages: AsyncIterable<string>): AsyncIterable<Delta> {
  for await (const message of messages) {
    let frame: { deltas?: Record<string, unknown>[] };
    try {
      frame = JSON.parse(message);
    } catch {
      continue;
    }
    for (const d of frame.deltas ?? []) {
      const view = wsDelta(d);
      if (view !== null) yield view;
    }
  }
}

/**
 * Map one global-channel WS JSON delta (Batch C `/v1/events/all`; `type`
 * discriminant, hex ids) to a {@link GlobalDelta}. An unrecognized `type` maps
 * to `"unknown"` — forward-compat: a future delta kind never throws.
 */
export function wsAllDelta(obj: Record<string, unknown>): GlobalDelta {
  const kind = obj.type as string | undefined;
  const seq = Number(obj.seq ?? 0);
  const str = (k: string): string | null =>
    typeof obj[k] === "string" ? (obj[k] as string) : null;
  const num = (k: string): number | null => (obj[k] != null ? Number(obj[k]) : null);
  const instanceId = str("instance_id") ?? ""; // "" pre-registration (honest)
  switch (kind) {
    case "run_registered":
      return new GlobalDelta(
        seq,
        "run_registered",
        instanceId,
        null,
        null,
        null,
        null,
        null,
        null,
        str("recipe_fingerprint"),
        num("registered_unix_ms"),
      );
    case "committed":
      return new GlobalDelta(
        seq,
        "committed",
        instanceId,
        str("mote_id"),
        str("result_ref"),
        ndClassFromTag(str("nd_class")),
      );
    case "failed":
      return new GlobalDelta(
        seq,
        "failed",
        instanceId,
        str("mote_id"),
        null,
        null,
        num("reason_class"),
      );
    case "repudiated":
      return new GlobalDelta(
        seq,
        "repudiated",
        instanceId,
        null,
        null,
        null,
        null,
        str("target_mote_id"),
        num("target_committed_seq"),
      );
    case "effect_staged":
      return new GlobalDelta(seq, "effect_staged", instanceId, str("mote_id"));
    default:
      return new GlobalDelta(seq, "unknown", instanceId);
  }
}

/** Parse a stream of global-channel WS JSON frame messages into deltas. */
export async function* wsAllDeltasFromMessages(
  messages: AsyncIterable<string>,
): AsyncIterable<GlobalDelta> {
  for await (const message of messages) {
    let frame: { deltas?: Record<string, unknown>[] };
    try {
      frame = JSON.parse(message);
    } catch {
      continue;
    }
    for (const d of frame.deltas ?? []) {
      yield wsAllDelta(d);
    }
  }
}

/**
 * Bridge an event-emitter-style WebSocket into an async iterable of message
 * strings — the platform-neutral core shared by the Node (`ws`) and browser
 * (`WebSocket`) paths. `wire` registers the message/close/error callbacks; `close`
 * tears the socket down when iteration ends.
 */
export async function* socketMessages(
  wire: (onMsg: (m: string) => void, onClose: () => void, onErr: (e: unknown) => void) => void,
  close: () => void,
): AsyncIterable<string> {
  const queue: string[] = [];
  let wake: (() => void) | null = null;
  let done = false;
  let err: unknown = null;
  const bump = () => {
    if (wake) {
      wake();
      wake = null;
    }
  };
  wire(
    (m) => {
      queue.push(m);
      bump();
    },
    () => {
      done = true;
      bump();
    },
    (e) => {
      err = e;
      done = true;
      bump();
    },
  );
  try {
    for (;;) {
      if (queue.length > 0) {
        yield queue.shift() as string;
        continue;
      }
      if (done) break;
      await new Promise<void>((r) => {
        wake = r;
      });
    }
    if (err !== null) {
      throw new KxConnectError(`websocket error: ${String(err)}`);
    }
  } finally {
    close();
  }
}
