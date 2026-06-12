/**
 * The Batch A model-discovery view — one `ListModels` entry. Display/discovery
 * ONLY (SN-8): model *selection* stays a recipe ENUM free-param validated
 * server-side at binding; nothing here authorizes a model route. An FFI-free
 * gateway answers with an EMPTY list (honest, not an error).
 */

import type { ModelSummary as PbModelSummary } from "./gen/kortecx/v1/gateway_pb.js";

/** One discoverable model on the connected gateway. */
export class ModelSummary {
  constructor(
    /** The model id a recipe `model` ENUM free-param accepts. */
    readonly modelId: string,
    /** Display modality strings: `"text" | "image" | "audio" | "video"`. */
    readonly modalities: readonly string[],
    /** Host-synthesized display prose (GGUF name / file stem) — never identity. */
    readonly description: string,
    /** `true` iff this model backs the live serve loop right now. */
    readonly serving: boolean,
    /** The served context window in tokens. */
    readonly contextLen: number,
  ) {}

  static fromProto(m: PbModelSummary): ModelSummary {
    return new ModelSummary(m.modelId, m.modalities, m.description, m.serving, m.contextLen);
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      model_id: this.modelId,
      modalities: [...this.modalities],
      description: this.description,
      serving: this.serving,
      context_len: this.contextLen,
    };
  }
}
