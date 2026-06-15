/**
 * Live token-stream view (PR-4.2 / T-STREAM1).
 *
 * One {@link TokenChunk} is the NEW model bytes for one decode step. Concatenating
 * `text` across a stream in `seq` order rebuilds the completion — byte-identical
 * to the committed `result_ref` (the durable authority). The stream is ADVISORY +
 * out-of-band: display-only, never an authority/identity input. `done` marks the
 * terminal chunk; the stream ends after it.
 */

import type { TokenChunk as TokenChunkProto } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

export class TokenChunk {
  constructor(
    /** Broker-assigned per-mote monotone counter (advisory; NOT a journal seq). */
    readonly seq: number,
    /** The model mote this chunk belongs to (64-hex). */
    readonly moteId: string,
    /** The NEW detokenized text for this step (UTF-8; may be lossy at a token
     *  boundary — cosmetic, reconciled when the committed result is fetched). */
    readonly text: string,
    /** True on the terminal chunk (generation ended). */
    readonly done: boolean,
    /** The raw piece bytes (gRPC path only; empty on the WS/JSON path). */
    readonly bytes: Uint8Array = new Uint8Array(),
  ) {}

  /** From the native gRPC proto chunk (raw bytes preserved for exact concat). */
  static fromProto(c: TokenChunkProto): TokenChunk {
    return new TokenChunk(
      Number(c.seq),
      encode(c.moteId),
      new TextDecoder().decode(c.textPiece),
      c.done,
      c.textPiece,
    );
  }

  /** From the WS JSON chunk (`text_piece` already a lossy-UTF-8 string). */
  static fromWs(obj: Record<string, unknown>): TokenChunk {
    const str = (k: string): string => (typeof obj[k] === "string" ? (obj[k] as string) : "");
    return new TokenChunk(
      Number(obj.seq ?? 0),
      str("mote_id"),
      str("text_piece"),
      obj.done === true,
    );
  }
}
