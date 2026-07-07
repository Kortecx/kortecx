/**
 * The Settings server-info view — the resolved `GetServerInfo` configuration the
 * connected gateway is running (POC-1). Read by an authenticated caller; a DISPLAY
 * projection only (SN-8): nothing here is identity, a secret, or a digest input —
 * the TLS field is a POSTURE flag (whether in-binary TLS is on), never the key.
 *
 * Kept in its own module so `types.ts` / `common.ts` stay thin aggregators,
 * mirroring the Rust core's module-per-concern discipline and the sibling
 * {@link DatasetSummary} / {@link ModelSummary} views.
 */

import type { GetServerInfoResponse as PbGetServerInfoResponse } from "./gen/kortecx/v1/gateway_pb.js";

/** The resolved configuration of the connected `kx serve` gateway. */
export class ServerInfo {
  constructor(
    /** Resolved serve model id (`""` on a model-less serve). */
    readonly modelId: string,
    /** Resolved GGUF path (`""` on a model-less serve). */
    readonly modelPath: string,
    /** gRPC listener bind `addr:port`. */
    readonly listenAddr: string,
    /** R5 WebSocket live-event bridge `addr:port`. */
    readonly wsAddr: string,
    /** Embedded console `addr:port` (`""` if disabled). */
    readonly consoleAddr: string,
    /** Prometheus `/metrics` `addr:port` (`""` if off). */
    readonly metricsAddr: string,
    /** Content store directory. */
    readonly contentRoot: string,
    /** SQLite journal path. */
    readonly journalPath: string,
    /** Durable catalog directory. */
    readonly catalogDir: string,
    /** Worker lease batch size. */
    readonly maxLease: number,
    /** Fail-closed PutContent payload cap (bytes). */
    readonly contentMaxBytes: number,
    /** Browser CORS allowlist (display; empty = deny). */
    readonly corsOrigins: readonly string[],
    /** Serving in-binary TLS (POSTURE — never the key). */
    readonly tlsEnabled: boolean,
    /** `"deny-all" | "dev-local" | "token"` (label only). */
    readonly authMode: string,
    /** Datasets/RAG data-plane available. */
    readonly featureHnsw: boolean,
    /** Live model inference available. */
    readonly featureInference: boolean,
    /** Embedded console available. */
    readonly featureConsole: boolean,
    /** The serve model is image-capable. */
    readonly featureVision: boolean,
    /** A JSONL operator audit log is configured. */
    readonly auditLogEnabled: boolean,
    /** T-MULTI-ELEMENT-TOOLCALLS: the server's DEFAULT agentic model-turn budget (also
     *  the hard ceiling); a run overrides it per-invocation via `maxTurns`. */
    readonly reactMaxTurns: number = 0,
    /** T-MULTI-ELEMENT-TOOLCALLS: the server's DEFAULT total tool-call budget (a turn
     *  may fire several at once, so independent of `reactMaxTurns`); overridable per-run. */
    readonly reactMaxToolCalls: number = 0,
    /** PR-B: the configured datasets/RAG embed model id (`""` on a model-less serve). */
    readonly embedModelId: string = "",
    /** Model Control v2: the active default model (`""` ⇒ the primary; advisory). */
    readonly activeModelId: string = "",
    /** Model Control v2: operator-enabled model downloads (`KX_SERVE_ALLOW_MODEL_PULL`). */
    readonly allowModelPull: boolean = false,
    /** RC4a: the configured embedder is a decoder LLM (weak embeddings) — advisory. */
    readonly embedModelIsDecoder: boolean = false,
    /** The resolved embedded-worker POOL size (`--workers` / `KX_WORKERS` /
     *  `KX_SERVE_WORKER_POOL`; `1` = single worker, `>1` runs Pure/IO/tool Motes
     *  concurrently). `0` from an old server ⇒ treat as `1` (see {@link effectiveWorkerPool}). */
    readonly workerPool: number = 0,
  ) {}

  /** The pool size to display: `max(1, workerPool)` (an old server sends 0). */
  get effectiveWorkerPool(): number {
    return Math.max(1, this.workerPool);
  }

  static fromProto(r: PbGetServerInfoResponse): ServerInfo {
    return new ServerInfo(
      r.modelId,
      r.modelPath,
      r.listenAddr,
      r.wsAddr,
      r.consoleAddr,
      r.metricsAddr,
      r.contentRoot,
      r.journalPath,
      r.catalogDir,
      Number(r.maxLease),
      Number(r.contentMaxBytes),
      r.corsOrigins,
      r.tlsEnabled,
      r.authMode,
      r.featureHnsw,
      r.featureInference,
      r.featureConsole,
      r.featureVision,
      r.auditLogEnabled,
      r.reactMaxTurns,
      r.reactMaxToolCalls,
      r.embedModelId,
      r.activeModelId,
      r.allowModelPull,
      r.embedModelIsDecoder,
      Number(r.workerPool),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      model_id: this.modelId,
      model_path: this.modelPath,
      listen_addr: this.listenAddr,
      ws_addr: this.wsAddr,
      console_addr: this.consoleAddr,
      metrics_addr: this.metricsAddr,
      content_root: this.contentRoot,
      journal_path: this.journalPath,
      catalog_dir: this.catalogDir,
      max_lease: this.maxLease,
      content_max_bytes: this.contentMaxBytes,
      cors_origins: [...this.corsOrigins],
      tls_enabled: this.tlsEnabled,
      auth_mode: this.authMode,
      feature_hnsw: this.featureHnsw,
      feature_inference: this.featureInference,
      feature_console: this.featureConsole,
      feature_vision: this.featureVision,
      audit_log_enabled: this.auditLogEnabled,
      react_max_turns: this.reactMaxTurns,
      react_max_tool_calls: this.reactMaxToolCalls,
      embed_model_id: this.embedModelId,
      active_model_id: this.activeModelId,
      allow_model_pull: this.allowModelPull,
      embed_model_is_decoder: this.embedModelIsDecoder,
      worker_pool: this.workerPool,
    };
  }
}
