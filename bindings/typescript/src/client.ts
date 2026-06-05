/**
 * The transport-agnostic core client. `KxClientBase` holds the Connect client and
 * the eight RPCs plus the high-level `invoke(..., { wait: true })` "runtime as a
 * function" path. The Node (`node.ts`) and browser (`web.ts`) entrypoints subclass
 * it to supply a gRPC vs gRPC-web transport and the platform WebSocket — that is
 * the ONLY thing that differs between them.
 *
 * Identity is server-derived (SN-8): the client sends a *credential* (a bearer
 * token), never a claimed identity, and never computes an id.
 */

import type { MessageInitShape } from "@bufbuild/protobuf";
import { createClient } from "@connectrpc/connect";
import type { Client, Transport } from "@connectrpc/connect";
import { KxRunFailed, KxWaitTimeout, rpc } from "./errors.js";
import { streamDeltas, wsDeltasFromMessages, wsUrl } from "./events.js";
import { KxGateway, type SubmitRunRequestSchema } from "./gen/kortecx/v1/gateway_pb.js";
import { INSTANCE_LEN, REF_LEN, asBytes, encode } from "./hexids.js";
import { Result, Run } from "./run.js";
import { type Args, encodeArgs } from "./transport.js";
import { type Delta, Projection, SignatureSummary } from "./types.js";
import { type WaitMode, type WaitOutcome, eventsResult, pollAny, pollResult } from "./wait.js";

/** An id argument: hex string OR raw server-derived bytes. */
export type Id = string | Uint8Array;

/** Options for constructing a client (the transport differs per entrypoint). */
export interface KxClientOptions {
  /** A bearer token (mutually exclusive with `tokenFile`). */
  token?: string;
  /** Read the bearer token from a file (Node only; throws in the browser). */
  tokenFile?: string;
  /** Override the auto-picked Connect transport. */
  transport?: Transport;
  /** Explicit WS bridge endpoint for `wsEvents` (else derived from the gRPC one). */
  wsEndpoint?: string;
}

/** Options for {@link KxClientBase.invoke}. */
export interface InvokeOptions {
  /** `false` (default) returns a {@link Run}; `true` blocks for the {@link Result}. */
  wait?: boolean;
  timeoutMs?: number;
  /** `"poll"` (default) | `"events"` (low-latency live subscription). */
  waitMode?: WaitMode;
  /** Write the committed payload to this file (Node only). */
  out?: string;
}

export abstract class KxClientBase {
  readonly endpoint: string;
  protected readonly token: string | undefined;
  protected readonly wsEndpoint: string | undefined;
  protected readonly grpc: Client<typeof KxGateway>;

  protected constructor(
    endpoint: string,
    transport: Transport,
    opts: { token?: string; wsEndpoint?: string },
  ) {
    this.endpoint = endpoint;
    this.token = opts.token;
    this.wsEndpoint = opts.wsEndpoint;
    this.grpc = createClient(KxGateway, transport);
  }

  /**
   * Bind a published recipe to `args` and run it. With `wait: true` blocks for the
   * committed {@link Result} (throwing {@link KxRunFailed} / {@link KxWaitTimeout}
   * on a failed / timed-out run); otherwise returns a {@link Run} handle.
   */
  async invoke(handle: string, args: Args, opts: InvokeOptions = {}): Promise<Run | Result> {
    const argBytes = encodeArgs(args);
    const resp = await rpc(this.grpc.invoke({ handle, args: argBytes }));
    const run = new Run(this, resp.instanceId, resp.terminalMoteId, resp.recipeFingerprint);
    if (!opts.wait) return run;
    const result = await this._awaitTerminal(
      resp.instanceId,
      resp.terminalMoteId,
      opts.timeoutMs ?? 120_000,
      opts.waitMode ?? "poll",
    );
    if (opts.out !== undefined && result.payload !== null) {
      await this.writeOut(opts.out, result.payload);
    }
    return result;
  }

  /**
   * Low-level propose-proxy submit (advanced; recipe authoring lives in the
   * runtime). Returns the run handle, or — with `wait: true` — the first
   * committed {@link Result}.
   */
  async submitRun(
    request: MessageInitShape<typeof SubmitRunRequestSchema>,
    opts: { wait?: boolean; timeoutMs?: number } = {},
  ): Promise<{ instanceId: Uint8Array; recipeFingerprint: Uint8Array } | Result> {
    const handle = await rpc(this.grpc.submitRun(request));
    if (!opts.wait) return handle;
    const outcome = await pollAny(this.grpc, handle.instanceId, opts.timeoutMs ?? 120_000);
    return this._finish(outcome);
  }

  async getProjection(instanceId: Id, opts: { atSeq?: bigint } = {}): Promise<Projection> {
    const inst = asBytes(instanceId, INSTANCE_LEN);
    const view = await rpc(this.grpc.getProjection({ instanceId: inst, atSeq: opts.atSeq }));
    return Projection.fromProto(view);
  }

  async getContent(ref: Id, instanceId: Id): Promise<Uint8Array> {
    const cref = asBytes(ref, REF_LEN);
    const inst = asBytes(instanceId, INSTANCE_LEN);
    const blob = await rpc(this.grpc.getContent({ contentRef: cref, instanceId: inst }));
    return blob.payload;
  }

  /** Native gRPC server-streaming event tail (Node + browser via Connect). */
  streamEvents(
    instanceId: Id,
    opts: { since?: bigint; follow?: boolean; signal?: AbortSignal } = {},
  ): AsyncIterable<Delta> {
    const inst = asBytes(instanceId, INSTANCE_LEN);
    return streamDeltas(this.grpc, inst, opts.since ?? 0n, opts.follow ?? false, opts.signal);
  }

  /** Consume the live tail over the R5 WebSocket bridge (browser-friendly). */
  async *wsEvents(
    instanceId: Id,
    opts: { since?: bigint; wsEndpoint?: string } = {},
  ): AsyncIterable<Delta> {
    const inst = asBytes(instanceId, INSTANCE_LEN);
    const url = wsUrl(
      this.endpoint,
      opts.wsEndpoint ?? this.wsEndpoint,
      encode(inst),
      opts.since ?? 0n,
    );
    yield* wsDeltasFromMessages(this.openWsMessages(url, this.token));
  }

  async listSignatures(): Promise<SignatureSummary[]> {
    const resp = await rpc(this.grpc.listSignatures({}));
    return resp.signatures.map((s) => SignatureSummary.fromProto(s));
  }

  async getSignature(signatureId: Id): Promise<Uint8Array> {
    const sid = asBytes(signatureId, REF_LEN);
    const resp = await rpc(this.grpc.getSignature({ signatureId: sid }));
    return resp.manifest;
  }

  async registerSignature(manifest: Uint8Array): Promise<string> {
    const resp = await rpc(this.grpc.registerSignature({ manifest }));
    return encode(resp.signatureId);
  }

  /** Connect transports manage their own connections; there is nothing to close. */
  close(): void {
    /* no-op (kept for API symmetry with the Python SDK). */
  }

  /** Wait plumbing — shared by `invoke` and {@link Run.wait}. */
  async _awaitTerminal(
    instance: Uint8Array,
    terminal: Uint8Array,
    timeoutMs: number,
    mode: WaitMode,
  ): Promise<Result> {
    const outcome =
      mode === "events"
        ? await eventsResult(this.grpc, instance, terminal, timeoutMs)
        : await pollResult(this.grpc, instance, terminal, timeoutMs);
    return this._finish(outcome);
  }

  protected _finish(outcome: WaitOutcome): Result {
    const result = Result.fromOutcome(outcome);
    if (outcome.state === "FAILED") {
      throw new KxRunFailed("the run's terminal Mote failed", {
        instanceId: result.instanceId,
        terminalMoteId: result.terminalMoteId || undefined,
      });
    }
    if (outcome.state === "RUNNING") {
      throw new KxWaitTimeout(
        "run still in progress (timed out); resume with getProjection / events",
        { instanceId: result.instanceId, terminalMoteId: result.terminalMoteId || undefined },
      );
    }
    return result;
  }

  /** Platform hook: open the R5 WS bridge and yield raw JSON frame messages. */
  protected abstract openWsMessages(url: string, token: string | undefined): AsyncIterable<string>;

  /** Platform hook: write the committed payload to a file (Node) / refuse (browser). */
  protected abstract writeOut(path: string, bytes: Uint8Array): Promise<void>;
}
