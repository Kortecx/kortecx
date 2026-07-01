/**
 * RC5a durable agentic MEMORY views — a stored memory, a recall hit, and a store
 * receipt, as surfaced by `StoreMemory` / `ListMemories` / `RecallMemory` /
 * `ForgetMemory`.
 *
 * Cross-run, per-namespace memory: what an agent LEARNED in one run and can RECALL
 * in a later one. SN-8: a recall hit's `score` is DISPLAY-ONLY — never an identity
 * input; the durable result is the ordered `memoryId` SET, matched by EXACT hash.
 * Every memory is scoped to the caller's own principal (server-derived).
 */

import type {
  DecayCandidate as PbDecayCandidate,
  DecayMemoryResponse as PbDecayMemoryResponse,
  MemoryHit as PbMemoryHit,
  MemoryStatsResponse as PbMemoryStatsResponse,
  MemorySummary as PbMemorySummary,
  StoreMemoryResponse as PbStoreMemoryResponse,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/**
 * The kind of a memory (metadata; does not change indexing) — re-exported from the
 * generated proto enum so it is type-compatible with the wire. `UNSPECIFIED` ⇒
 * `SEMANTIC` (the default).
 */
export { MemoryKind } from "./gen/kortecx/v1/gateway_pb.js";

/** One stored memory (the episodic-log view from `ListMemories`). */
export class Memory {
  constructor(
    /** Hex of the content-addressed id (the citation key). */
    readonly memoryId: string,
    readonly content: Uint8Array,
    /** `"semantic"` | `"episodic"`. */
    readonly kind: string,
    /** Hex of the run that wrote it (all-zero = operator/SDK write). */
    readonly instanceId: string,
    /** Unix-ms write time (display only; off every hash). */
    readonly createdMs: number,
    readonly dim: number,
    /** RC5b: recall count (salience; display only). */
    readonly accessCount: number = 0,
    /** RC5b: last recall time, unix-ms (0 = never). */
    readonly lastAccessedMs: number = 0,
    /** RC5b: decay tombstone time, unix-ms (0 = live; >0 = decayed, restorable). */
    readonly tombstonedMs: number = 0,
  ) {}

  static fromProto(m: PbMemorySummary): Memory {
    return new Memory(
      encode(m.memoryId),
      m.content,
      m.kind,
      encode(m.instanceId),
      Number(m.createdMs),
      m.dim,
      m.accessCount,
      Number(m.lastAccessedMs),
      Number(m.tombstonedMs),
    );
  }

  /** The remembered bytes decoded as UTF-8 (best-effort). */
  get text(): string {
    return new TextDecoder().decode(this.content);
  }

  /** True if this memory has been decayed (soft-tombstoned; restorable). */
  get isDecayed(): boolean {
    return this.tombstonedMs > 0;
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      memory_id: this.memoryId,
      text: this.text,
      kind: this.kind,
      instance_id: this.instanceId,
      created_ms: this.createdMs,
      dim: this.dim,
      access_count: this.accessCount,
      last_accessed_ms: this.lastAccessedMs,
      tombstoned_ms: this.tombstonedMs,
    };
  }
}

/** One recall hit: the content-addressed ref (hex), the bytes, and the DISPLAY-ONLY
 *  similarity score (SN-8 — never an identity input). */
export class MemoryHit {
  constructor(
    readonly memoryId: string,
    readonly content: Uint8Array,
    readonly score: number,
  ) {}

  static fromProto(h: PbMemoryHit): MemoryHit {
    return new MemoryHit(encode(h.memoryId), h.content, h.score);
  }

  /** The recalled bytes decoded as UTF-8 (best-effort). */
  get text(): string {
    return new TextDecoder().decode(this.content);
  }

  toJSON() {
    return { memory_id: this.memoryId, score: this.score, text: this.text };
  }
}

/** The outcome of a `StoreMemory` (content-addressed, idempotent). */
export class StoreResult {
  constructor(
    readonly memoryId: string,
    /** `false` ⇒ a content-addressed dedup hit. */
    readonly inserted: boolean,
    readonly dim: number,
  ) {}

  static fromProto(r: PbStoreMemoryResponse): StoreResult {
    return new StoreResult(encode(r.memoryId), r.inserted, r.dim);
  }

  toJSON() {
    return { memory_id: this.memoryId, inserted: this.inserted, dim: this.dim };
  }
}

/** One memory a decay policy matched (RC5b) — a reversible soft-tombstone, never a
 *  hard delete. */
export class DecayCandidate {
  constructor(
    readonly memoryId: string,
    readonly content: Uint8Array,
    readonly kind: string,
    readonly createdMs: number,
    readonly accessCount: number,
    readonly lastAccessedMs: number,
    readonly ageDays: number,
  ) {}

  static fromProto(c: PbDecayCandidate): DecayCandidate {
    return new DecayCandidate(
      encode(c.memoryId),
      c.content,
      c.kind,
      Number(c.createdMs),
      c.accessCount,
      Number(c.lastAccessedMs),
      c.ageDays,
    );
  }

  /** The memory bytes decoded as UTF-8 (best-effort). */
  get text(): string {
    return new TextDecoder().decode(this.content);
  }

  toJSON() {
    return {
      memory_id: this.memoryId,
      text: this.text,
      kind: this.kind,
      created_ms: this.createdMs,
      access_count: this.accessCount,
      last_accessed_ms: this.lastAccessedMs,
      age_days: this.ageDays,
    };
  }
}

/** The outcome of a `DecayMemory` sweep (RC5b). `dryRun` ⇒ a preview that evicted
 *  nothing; evictions are reversible via `restoreMemory`. */
export class DecayReport {
  constructor(
    readonly candidates: DecayCandidate[],
    readonly wouldEvict: number,
    readonly evicted: number,
    readonly kept: number,
    readonly dryRun: boolean,
  ) {}

  static fromProto(r: PbDecayMemoryResponse): DecayReport {
    return new DecayReport(
      r.candidates.map((c) => DecayCandidate.fromProto(c)),
      r.wouldEvict,
      r.evicted,
      r.kept,
      r.dryRun,
    );
  }

  toJSON() {
    return {
      candidates: this.candidates.map((c) => c.toJSON()),
      would_evict: this.wouldEvict,
      evicted: this.evicted,
      kept: this.kept,
      dry_run: this.dryRun,
    };
  }
}

/** Namespace memory statistics (RC5b). */
export class MemoryStats {
  constructor(
    readonly total: number,
    readonly semantic: number,
    readonly episodic: number,
    readonly tombstoned: number,
    readonly dim: number,
    readonly embedFingerprint: string,
    readonly oldestMs: number,
    readonly newestMs: number,
    readonly namespace: string,
  ) {}

  static fromProto(s: PbMemoryStatsResponse): MemoryStats {
    return new MemoryStats(
      s.total,
      s.semantic,
      s.episodic,
      s.tombstoned,
      s.dim,
      s.embedFingerprint,
      Number(s.oldestMs),
      Number(s.newestMs),
      s.namespace,
    );
  }

  toJSON() {
    return {
      total: this.total,
      semantic: this.semantic,
      episodic: this.episodic,
      tombstoned: this.tombstoned,
      dim: this.dim,
      embed_fingerprint: this.embedFingerprint,
      oldest_ms: this.oldestMs,
      newest_ms: this.newestMs,
      namespace: this.namespace,
    };
  }
}
