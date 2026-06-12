/**
 * Idiomatic, read-only views over the generated protobuf messages.
 *
 * These wrap the raw `kortecx.v1` messages with hex ids and stable display names,
 * mirroring the `kx` CLI's rendering (`format.rs`) and the Python SDK so the
 * surfaces agree field-for-field (`toJSON()` == the CLI `--json` shape). An
 * out-of-range enum renders `UNKNOWN` — never a crash, never a silent mislabel.
 */

import type {
  EventDelta,
  EventFrame,
  GlobalEventDelta,
  MoteSnapshot,
  SignatureSummary as PbSignatureSummary,
  ProjectionView,
} from "./gen/kortecx/v1/gateway_pb.js";
import { MoteSnapshotState } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";
import { ParentEdge } from "./parents.js";

const STATE_NAMES: Record<number, string> = {
  [MoteSnapshotState.PENDING]: "PENDING",
  [MoteSnapshotState.SCHEDULED]: "SCHEDULED",
  [MoteSnapshotState.COMMITTED]: "COMMITTED",
  [MoteSnapshotState.FAILED]: "FAILED",
  [MoteSnapshotState.REPUDIATED]: "REPUDIATED",
  [MoteSnapshotState.INCONSISTENT]: "INCONSISTENT",
};

/** Map a `MoteSnapshotState` discriminant to a stable name (`UNKNOWN` if new). */
export function stateName(state: number): string {
  return STATE_NAMES[state] ?? "UNKNOWN";
}

export function isCommitted(state: number): boolean {
  return state === MoteSnapshotState.COMMITTED;
}

/** True for a not-yet-terminal state (keep polling). */
export function isPending(state: number): boolean {
  return state === MoteSnapshotState.PENDING || state === MoteSnapshotState.SCHEDULED;
}

/** One Mote in a run's projection, with hex ids + a display state. */
export class MoteView {
  constructor(
    readonly moteId: string,
    readonly state: string,
    readonly stateCode: number,
    readonly ndClass: number,
    readonly promotion: number,
    readonly resultRef: string | null,
    readonly moteDefHash: string,
    readonly committedSeq: number | null,
    readonly anomaly: number | null,
    /**
     * Inbound DAG edges (server-derived). Trailing + defaulted so existing
     * positional callers keep working; deliberately NOT in `toJSON()` (the CLI
     * `projection --json` shape carries no parents — byte-parity is load-bearing).
     */
    readonly parents: readonly ParentEdge[] = [],
  ) {}

  static fromProto(m: MoteSnapshot): MoteView {
    return new MoteView(
      encode(m.moteId),
      stateName(m.state),
      m.state,
      m.ndClass,
      m.promotion,
      m.resultRef !== undefined ? encode(m.resultRef) : null,
      encode(m.moteDefHash),
      m.committedSeq !== undefined ? Number(m.committedSeq) : null,
      m.anomaly !== undefined ? m.anomaly : null,
      m.parents.map((p) => ParentEdge.fromProto(p)),
    );
  }

  /** The CLI `--json` mote shape (ints for nd_class/promotion/anomaly). */
  toJSON(): Record<string, unknown> {
    return {
      mote_id: this.moteId,
      state: this.state,
      nd_class: this.ndClass,
      promotion: this.promotion,
      result_ref: this.resultRef,
      committed_seq: this.committedSeq,
      anomaly: this.anomaly,
    };
  }
}

/** A run rendered as a DAG of Mote states (a fold-frontier snapshot). */
export class Projection {
  constructor(
    readonly instanceId: string,
    readonly recipeFingerprint: string,
    readonly currentSeq: number,
    readonly motes: MoteView[],
  ) {}

  static fromProto(view: ProjectionView): Projection {
    return new Projection(
      encode(view.instanceId),
      encode(view.recipeFingerprint),
      Number(view.currentSeq),
      view.motes.map((m) => MoteView.fromProto(m)),
    );
  }

  /** Find a Mote by its hex id (`null` if absent at this frontier). */
  mote(moteId: string): MoteView | null {
    return this.motes.find((m) => m.moteId === moteId) ?? null;
  }

  get committed(): MoteView[] {
    return this.motes.filter((m) => isCommitted(m.stateCode));
  }

  /** The CLI `--json` projection shape (for parity / scripting). */
  toJSON(): Record<string, unknown> {
    return {
      instance_id: this.instanceId,
      recipe_fingerprint: this.recipeFingerprint,
      current_seq: this.currentSeq,
      motes: this.motes.map((m) => m.toJSON()),
    };
  }
}

/**
 * One event delta (committed / failed / repudiated / effect_staged). `kind` is
 * the stable lowercase discriminant; fields not relevant to the kind are `null`.
 * Mirrors the CLI `render_delta` / WS-bridge JSON shape.
 */
export class Delta {
  constructor(
    readonly seq: number,
    readonly kind: string,
    readonly moteId: string | null = null,
    readonly resultRef: string | null = null,
    readonly ndClass: number | null = null,
    readonly reasonClass: number | null = null,
    readonly targetMoteId: string | null = null,
    readonly targetCommittedSeq: number | null = null,
  ) {}

  /** Build a view, or `null` for a delta with no recognized kind (skip). */
  static fromProto(d: EventDelta): Delta | null {
    const seq = Number(d.seq);
    switch (d.kind.case) {
      case "committed": {
        const c = d.kind.value;
        return new Delta(seq, "committed", encode(c.moteId), encode(c.resultRef), c.ndClass);
      }
      case "failed": {
        const f = d.kind.value;
        return new Delta(seq, "failed", encode(f.moteId), null, null, f.reasonClass);
      }
      case "repudiated": {
        const r = d.kind.value;
        return new Delta(
          seq,
          "repudiated",
          null,
          null,
          null,
          null,
          encode(r.targetMoteId),
          Number(r.targetCommittedSeq),
        );
      }
      case "effectStaged": {
        const e = d.kind.value;
        return new Delta(seq, "effect_staged", encode(e.moteId));
      }
      default:
        return null;
    }
  }

  toJSON(): Record<string, unknown> {
    const out: Record<string, unknown> = { seq: this.seq, kind: this.kind };
    if (this.moteId !== null) out.mote_id = this.moteId;
    if (this.resultRef !== null) out.result_ref = this.resultRef;
    if (this.ndClass !== null) out.nd_class = this.ndClass;
    if (this.reasonClass !== null) out.reason_class = this.reasonClass;
    if (this.targetMoteId !== null) out.target_mote_id = this.targetMoteId;
    if (this.targetCommittedSeq !== null) out.target_committed_seq = this.targetCommittedSeq;
    return out;
  }
}

/** One `EventFrame`: a batch of deltas + the resume cursor + caught-up flag. */
export class Frame {
  constructor(
    readonly seq: number,
    readonly deltas: Delta[],
    readonly nextSeq: number,
    readonly journalBoundary: boolean,
  ) {}

  static fromProto(f: EventFrame): Frame {
    const deltas: Delta[] = [];
    for (const d of f.deltas) {
      const view = Delta.fromProto(d);
      if (view !== null) deltas.push(view);
    }
    return new Frame(Number(f.seq), deltas, Number(f.nextSeq), f.journalBoundary);
  }
}

/**
 * One operator-global event delta (Batch C `StreamAllEvents`): the per-run
 * {@link Delta} kinds PLUS `run_registered` (a run came into existence), each
 * stamped with its watermark `instanceId` attribution (`""` before any
 * registration — honest, never fabricated). Unlike {@link Delta.fromProto},
 * an unrecognized kind maps to `"unknown"` rather than being skipped — the
 * global tail narrates everything, it never throws and never silently drops.
 */
export class GlobalDelta {
  constructor(
    readonly seq: number,
    readonly kind: string,
    readonly instanceId: string,
    readonly moteId: string | null = null,
    readonly resultRef: string | null = null,
    readonly ndClass: number | null = null,
    readonly reasonClass: number | null = null,
    readonly targetMoteId: string | null = null,
    readonly targetCommittedSeq: number | null = null,
    readonly recipeFingerprint: string | null = null,
    readonly registeredUnixMs: number | null = null,
  ) {}

  /** Build a view; a delta with no recognized kind becomes `"unknown"` (never `null`). */
  static fromProto(d: GlobalEventDelta): GlobalDelta {
    const seq = Number(d.seq);
    const instanceId = encode(d.instanceId); // EMPTY bytes pre-registration → ""
    switch (d.kind.case) {
      case "committed": {
        const c = d.kind.value;
        return new GlobalDelta(
          seq,
          "committed",
          instanceId,
          encode(c.moteId),
          encode(c.resultRef),
          c.ndClass,
        );
      }
      case "failed": {
        const f = d.kind.value;
        return new GlobalDelta(
          seq,
          "failed",
          instanceId,
          encode(f.moteId),
          null,
          null,
          f.reasonClass,
        );
      }
      case "repudiated": {
        const r = d.kind.value;
        return new GlobalDelta(
          seq,
          "repudiated",
          instanceId,
          null,
          null,
          null,
          null,
          encode(r.targetMoteId),
          Number(r.targetCommittedSeq),
        );
      }
      case "effectStaged": {
        const e = d.kind.value;
        return new GlobalDelta(seq, "effect_staged", instanceId, encode(e.moteId));
      }
      case "runRegistered": {
        const rr = d.kind.value;
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
          encode(rr.recipeFingerprint),
          Number(rr.registeredUnixMs),
        );
      }
      default:
        return new GlobalDelta(seq, "unknown", instanceId);
    }
  }

  toJSON(): Record<string, unknown> {
    const out: Record<string, unknown> = {
      seq: this.seq,
      kind: this.kind,
      instance_id: this.instanceId,
    };
    if (this.moteId !== null) out.mote_id = this.moteId;
    if (this.resultRef !== null) out.result_ref = this.resultRef;
    if (this.ndClass !== null) out.nd_class = this.ndClass;
    if (this.reasonClass !== null) out.reason_class = this.reasonClass;
    if (this.targetMoteId !== null) out.target_mote_id = this.targetMoteId;
    if (this.targetCommittedSeq !== null) out.target_committed_seq = this.targetCommittedSeq;
    if (this.recipeFingerprint !== null) out.recipe_fingerprint = this.recipeFingerprint;
    if (this.registeredUnixMs !== null) out.registered_unix_ms = this.registeredUnixMs;
    return out;
  }
}

/** One registered task signature (id + name). */
export class SignatureSummary {
  constructor(
    readonly signatureId: string,
    readonly name: string,
  ) {}

  static fromProto(s: PbSignatureSummary): SignatureSummary {
    return new SignatureSummary(encode(s.signatureId), s.name);
  }
}
