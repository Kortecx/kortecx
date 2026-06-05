/**
 * User-facing run handles and results.
 *
 * {@link Result} is the one-object answer of a `wait` (mirrors the CLI
 * `render_wait` shape, so `Result.toJSON()` is comparable to `kx … --wait
 * --json`). {@link Run} is an ergonomic handle over a started run —
 * `.wait()`, `.projection()`, `.content()`, `.events()`.
 */

import type { KxClientBase } from "./client.js";
import { encode } from "./hexids.js";
import type { Delta, Projection } from "./types.js";
import type { WaitMode, WaitOutcome } from "./wait.js";

export type ResultState = "COMMITTED" | "FAILED" | "RUNNING";

/** The terminal outcome of a `wait` — server-derived ids + the result. */
export class Result {
  constructor(
    /** hex (16B). */
    readonly instanceId: string,
    /** hex (32B); "" on the submit-failure/timeout path. */
    readonly terminalMoteId: string,
    readonly state: ResultState,
    /** hex (32B) when committed. */
    readonly resultRef: string | null,
    readonly payload: Uint8Array | null,
  ) {}

  static fromOutcome(o: WaitOutcome): Result {
    return new Result(
      encode(o.instanceId),
      encode(o.terminalMoteId),
      o.state,
      o.resultRef !== undefined ? encode(o.resultRef) : null,
      o.payload ?? null,
    );
  }

  /** True iff the run committed. */
  get ok(): boolean {
    return this.state === "COMMITTED";
  }

  get timedOut(): boolean {
    return this.state === "RUNNING";
  }

  /** The committed result bytes (`null` if not committed / no result). */
  get bytes(): Uint8Array | null {
    return this.payload;
  }

  /** The committed result decoded as UTF-8 (`null` if not text / absent). */
  get text(): string | null {
    if (this.payload === null) return null;
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(this.payload);
    } catch {
      return null;
    }
  }

  /** The CLI `--wait --json` shape (parity with `render_wait` / the Python SDK). */
  toJSON(includePayload = true): Record<string, unknown> {
    const out: Record<string, unknown> = {
      instance_id: this.instanceId,
      terminal_mote_id: this.terminalMoteId,
      state: this.state,
    };
    if (this.resultRef !== null) out.result_ref = this.resultRef;
    if (this.timedOut) out.timed_out = true;
    if (this.payload !== null) {
      out.result_len = this.payload.length;
      if (includePayload) {
        const t = this.text;
        if (t !== null) out.result_utf8 = t;
        out.result_hex = encode(this.payload);
      }
    }
    return out;
  }
}

/** A started run on a {@link KxClientBase}. */
export class Run {
  constructor(
    private readonly client: KxClientBase,
    private readonly _instance: Uint8Array,
    private readonly _terminal: Uint8Array,
    private readonly _fingerprint: Uint8Array,
  ) {}

  /** The run instance id (hex, 16B). */
  get instanceId(): string {
    return encode(this._instance);
  }

  /** The sink Mote whose committed result is this invocation's output (hex). */
  get terminalMoteId(): string {
    return encode(this._terminal);
  }

  get recipeFingerprint(): string {
    return encode(this._fingerprint);
  }

  get instanceIdBytes(): Uint8Array {
    return this._instance;
  }

  get terminalMoteIdBytes(): Uint8Array {
    return this._terminal;
  }

  /** Block until this run's terminal Mote commits (or fails / times out). */
  wait(opts: { timeoutMs?: number; mode?: WaitMode } = {}): Promise<Result> {
    return this.client._awaitTerminal(
      this._instance,
      this._terminal,
      opts.timeoutMs ?? 120_000,
      opts.mode ?? "poll",
    );
  }

  /** Alias for {@link wait} (read as "give me the result"). */
  result(opts: { timeoutMs?: number } = {}): Promise<Result> {
    return this.wait(opts);
  }

  projection(atSeq?: bigint): Promise<Projection> {
    return this.client.getProjection(this._instance, { atSeq });
  }

  content(ref: string | Uint8Array): Promise<Uint8Array> {
    return this.client.getContent(ref, this._instance);
  }

  events(opts: { since?: bigint; follow?: boolean } = {}): AsyncIterable<Delta> {
    return this.client.streamEvents(this._instance, opts);
  }
}
