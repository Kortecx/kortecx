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

/** Map one R5 WS JSON delta (`type` discriminant, hex ids) to a {@link Delta}. */
export function wsDelta(obj: Record<string, unknown>): Delta | null {
  const kind = obj.type as string | undefined;
  const seq = Number(obj.seq ?? 0);
  const str = (k: string): string | null =>
    typeof obj[k] === "string" ? (obj[k] as string) : null;
  const num = (k: string): number | null => (obj[k] != null ? Number(obj[k]) : null);
  switch (kind) {
    case "committed":
      return new Delta(seq, "committed", str("mote_id"), str("result_ref"));
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
      return new GlobalDelta(seq, "committed", instanceId, str("mote_id"), str("result_ref"));
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
