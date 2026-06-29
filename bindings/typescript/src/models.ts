/**
 * The Batch A model-discovery view — one `ListModels` entry. Display/discovery
 * ONLY (SN-8): model *selection* stays a recipe ENUM free-param validated
 * server-side at binding; nothing here authorizes a model route. An FFI-free
 * gateway answers with an EMPTY list (honest, not an error).
 */

import type {
  GetPullStatusResponse as PbGetPullStatusResponse,
  LoadModelResponse as PbLoadModelResponse,
  ModelSummary as PbModelSummary,
  OffloadModelResponse as PbOffloadModelResponse,
} from "./gen/kortecx/v1/gateway_pb.js";

/** One discoverable model on the connected gateway. */
export class ModelSummary {
  constructor(
    /** The model id a recipe `model` ENUM free-param accepts. */
    readonly modelId: string,
    /** Display modality strings: `"text" | "image" | "audio" | "video"`. */
    readonly modalities: readonly string[],
    /** Host-synthesized display prose (GGUF name / file stem) — never identity. */
    readonly description: string,
    /** `true` iff this model is the PRIMARY/default serve route. */
    readonly serving: boolean,
    /** The served context window in tokens. */
    readonly contextLen: number,
    /** POC-3: `true` iff the model is RESIDENT in RAM right now (live LRU). */
    readonly loaded: boolean = false,
    /** POC-3: the recipe handle to chat with THIS model (the routing key). */
    readonly chatHandle: string = "",
    /** The serving engine: `"kx-llamacpp" | "kx-ollama"` (empty on an old host). */
    readonly engine: string = "",
    /** PR-B: `true` iff this model is the server's configured datasets/RAG embedder. */
    readonly canEmbed: boolean = false,
    /** Model Control v2: provenance — `"local" | "ollama" | "pulled-ollama" | "pulled-url"`. */
    readonly source: string = "",
    /** Model Control v2: `true` iff this is the server's ACTIVE default (advisory). */
    readonly active: boolean = false,
    /** Model Control v2: the RAG-grounded chat recipe handle for THIS model (or ""). */
    readonly chatRagHandle: string = "",
    /** RC4a: `true` iff this (the configured embedder) is a decoder LLM (weak embeddings). */
    readonly embedIsDecoder: boolean = false,
  ) {}

  static fromProto(m: PbModelSummary): ModelSummary {
    return new ModelSummary(
      m.modelId,
      m.modalities,
      m.description,
      m.serving,
      m.contextLen,
      m.loaded,
      m.chatHandle,
      m.engine,
      m.canEmbed,
      m.source,
      m.active,
      m.chatRagHandle,
      m.embedIsDecoder,
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      model_id: this.modelId,
      modalities: [...this.modalities],
      description: this.description,
      serving: this.serving,
      context_len: this.contextLen,
      loaded: this.loaded,
      chat_handle: this.chatHandle,
      engine: this.engine,
      can_embed: this.canEmbed,
      source: this.source,
      active: this.active,
      chat_rag_handle: this.chatRagHandle,
      embed_is_decoder: this.embedIsDecoder,
    };
  }
}

/** Model Control v2: the progress of a `pullModel` download + registration. */
export class PullStatus {
  constructor(
    readonly modelId: string,
    /** `"resolving" | "downloading" | "verifying" | "registering" | "done" | "failed"`. */
    readonly phase: string,
    readonly bytesDownloaded: number = 0,
    readonly bytesTotal: number = 0,
    readonly detail: string = "",
  ) {}

  static fromProto(modelId: string, r: PbGetPullStatusResponse): PullStatus {
    const phases = [
      "unspecified",
      "resolving",
      "downloading",
      "verifying",
      "registering",
      "done",
      "failed",
    ];
    return new PullStatus(
      modelId,
      phases[r.phase] ?? "unspecified",
      Number(r.bytesDownloaded),
      Number(r.bytesTotal),
      r.detail,
    );
  }

  get isTerminal(): boolean {
    return this.phase === "done" || this.phase === "failed";
  }

  toJSON() {
    return {
      model_id: this.modelId,
      phase: this.phase,
      bytes_downloaded: this.bytesDownloaded,
      bytes_total: this.bytesTotal,
      detail: this.detail,
    };
  }
}

/** The outcome of a `loadModel` / `offloadModel` call (POC-3). */
export class ModelLifecycleResult {
  constructor(
    /** The model the op targeted. */
    readonly modelId: string,
    /** Residency AFTER the op (true after load, false after offload). */
    readonly loaded: boolean,
    /** Residency BEFORE the op (load: false ⇒ a cold load happened). */
    readonly wasResident: boolean,
  ) {}

  static fromLoad(r: PbLoadModelResponse): ModelLifecycleResult {
    return new ModelLifecycleResult(r.modelId, r.loaded, r.wasResident);
  }

  static fromOffload(r: PbOffloadModelResponse): ModelLifecycleResult {
    return new ModelLifecycleResult(r.modelId, r.loaded, r.wasResident);
  }

  toJSON() {
    return { model_id: this.modelId, loaded: this.loaded, was_resident: this.wasResident };
  }
}
