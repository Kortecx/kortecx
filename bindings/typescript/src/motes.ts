/**
 * The per-mote definition view — `GetMoteDetail` (Batch B). DISPLAY-ONLY
 * (SN-8): the capped definition summary the coordinator persisted at admission,
 * resolved by `mote_def_hash`; nothing here authorizes anything. A mote that
 * has not committed (or was admitted by a pre-Batch-B binary) answers
 * `defFound: false` honestly. Kept in its own module per the module-per-concern
 * discipline (the `runs.ts` pattern).
 */

import type {
  MoteConfigEntry as PbMoteConfigEntry,
  MoteDetail as PbMoteDetail,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** Display name for a wire `NdClass` discriminant (CLI-parity strings). */
export function ndClassName(nd: number): string {
  switch (nd) {
    case 1:
      return "PURE";
    case 2:
      return "READ_ONLY_NONDET";
    case 3:
      return "WORLD_MUTATING";
    default:
      return "UNKNOWN";
  }
}

/** Display name for a wire `EffectPattern` discriminant (CLI-parity strings). */
export function effectPatternName(ep: number): string {
  switch (ep) {
    case 1:
      return "IdempotentByConstruction";
    case 2:
      return "StageThenCommit";
    case 3:
      return "ValidateThenCommit";
    default:
      return "UNKNOWN";
  }
}

/** One capped config entry of a Mote definition (opaque display bytes). */
export class MoteConfigItem {
  constructor(
    readonly key: string,
    /** The (possibly truncated) raw value bytes. */
    readonly value: Uint8Array,
    readonly truncated: boolean,
    readonly fullLen: number,
  ) {}

  static fromProto(e: PbMoteConfigEntry): MoteConfigItem {
    return new MoteConfigItem(e.key, e.value, e.truncated, Number(e.fullLen));
  }

  /** A plain snake_case object (the CLI `--json` parity shape). */
  toJSON() {
    return {
      key: this.key,
      value_hex: encode(this.value),
      truncated: this.truncated,
      full_len: this.fullLen,
    };
  }
}

/** The capped, display-only definition summary of one Mote. */
export class MoteDetail {
  constructor(
    readonly moteId: string,
    /** Hex def hash; EMPTY string until the Mote commits. */
    readonly moteDefHash: string,
    /** `false`: uncommitted, or admitted by a pre-Batch-B binary. */
    readonly defFound: boolean,
    /** `"pure" | "model" | "exec" | "shaper" | "critic" | "react-turn"` (display). */
    readonly stepKind: string,
    readonly modelId: string,
    /** The instruction text (`config_subset["prompt"]`), capped server-side. */
    readonly prompt: string,
    readonly promptTruncated: boolean,
    readonly configSubset: MoteConfigItem[],
    /** Tool name → pinned version. */
    readonly toolContract: Record<string, string>,
    readonly logicRef: string,
    readonly ndClass: number,
    readonly effectPattern: number,
    /** Hex producer id this Mote critiques, or `undefined`. */
    readonly criticFor: string | undefined,
    readonly isTopologyShaper: boolean,
    readonly schemaVersion: number,
  ) {}

  static fromProto(d: PbMoteDetail): MoteDetail {
    return new MoteDetail(
      encode(d.moteId),
      encode(d.moteDefHash),
      d.defFound,
      d.stepKind,
      d.modelId,
      d.prompt,
      d.promptTruncated,
      d.configSubset.map((e) => MoteConfigItem.fromProto(e)),
      { ...d.toolContract },
      encode(d.logicRef),
      d.ndClass,
      d.effectPattern,
      d.criticFor === undefined ? undefined : encode(d.criticFor),
      d.isTopologyShaper,
      d.schemaVersion,
    );
  }

  /** Display name for {@link ndClass}. */
  get ndClassName(): string {
    return ndClassName(this.ndClass);
  }

  /** Display name for {@link effectPattern}. */
  get effectPatternName(): string {
    return effectPatternName(this.effectPattern);
  }

  /** A plain snake_case object — field-for-field the CLI `--json` shape (the
   *  tri-surface parity contract). */
  toJSON() {
    const tools: Record<string, string> = {};
    for (const key of Object.keys(this.toolContract).sort()) {
      const version = this.toolContract[key];
      if (version !== undefined) {
        tools[key] = version;
      }
    }
    return {
      mote_id: this.moteId,
      mote_def_hash: this.moteDefHash,
      def_found: this.defFound,
      step_kind: this.stepKind,
      model_id: this.modelId,
      prompt: this.prompt,
      prompt_truncated: this.promptTruncated,
      config_subset: this.configSubset.map((e) => e.toJSON()),
      tool_contract: tools,
      logic_ref: this.logicRef,
      nd_class: this.ndClassName,
      effect_pattern: this.effectPatternName,
      critic_for: this.criticFor ?? null,
      is_topology_shaper: this.isTopologyShaper,
      schema_version: this.schemaVersion,
    };
  }
}
