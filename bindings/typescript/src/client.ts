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
import { CaptureRecord, type CaptureRecordPage } from "./capture.js";
import type { Chain } from "./chains.js";
import { ContentItem, PutResult } from "./content.js";
import { DatasetHit, DatasetSummary, type IngestDoc, IngestResult } from "./datasets.js";
import { KxConnectError, KxRunFailed, KxUnimplemented, KxWaitTimeout, rpc } from "./errors.js";
import {
  streamAllDeltas,
  streamDeltas,
  wsAllDeltasFromMessages,
  wsAllUrl,
  wsDeltasFromMessages,
  wsUrl,
} from "./events.js";
import { type FeedbackInput, type FeedbackPage, FeedbackRow, ratingToProto } from "./feedback.js";
import {
  KxGateway,
  type SubmitRunRequestSchema,
  type SubmitWorkflowRequestSchema,
} from "./gen/kortecx/v1/gateway_pb.js";
import { AssetGrants } from "./grants.js";
import { INSTANCE_LEN, REF_LEN, asBytes, encode } from "./hexids.js";
import { ModelSummary } from "./models.js";
import { MoteDetail } from "./motes.js";
import { ReactTurn, type ReactTurnPage } from "./react.js";
import { RecipeForm, RecipeInfo, ScoredRecipe } from "./recipes.js";
import { ReplanRound, type ReplanRoundPage } from "./replan.js";
import { Result, Run } from "./run.js";
import { type RunPage, RunSummary } from "./runs.js";
import { TeamMembers, type TeamSummary, teamsFromProto } from "./teams.js";
import { type MoteTelemetryPage, MoteTelemetryRow } from "./telemetry.js";
import { BundleScore, type BundleSpec, ToolManifest, bundleSpecToProto } from "./toolscout.js";
import { type Args, encodeArgs } from "./transport.js";
import { type Delta, type GlobalDelta, Projection, SignatureSummary } from "./types.js";
import {
  type WaitMode,
  type WaitOutcome,
  eventsResult,
  pollAny,
  pollReactResult,
  pollResult,
} from "./wait.js";

/**
 * The canonical ReAct recipe handle. A react run has NO statically-known terminal
 * Mote (the gateway returns a run-salted turn-0 id that never commits, and the
 * settled Answer turn isn't known until the model emits it), so `invoke({ wait })`
 * on this handle settles via `ListReactTurns` instead of a terminal Mote (F13).
 */
export const REACT_RECIPE_HANDLE = "kx/recipes/react";

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
    const result =
      handle === REACT_RECIPE_HANDLE
        ? // F13: a react chain settles via ListReactTurns, not a terminal Mote.
          this._finish(
            await pollReactResult(
              this.grpc,
              resp.instanceId,
              resp.terminalMoteId,
              opts.timeoutMs ?? 120_000,
            ),
          )
        : await this._awaitTerminal(
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

  /**
   * Author a Tier-1 DAG (a {@link BlueprintBuilder}'s `build()`) and run it. The
   * server COMPILES the DAG, derives all identity, and builds every warrant from the
   * party's grants (SN-8) — the client sends only the topology + params. Returns the
   * run handle, or — with `wait: true` — the first committed {@link Result}. An old
   * gateway without the seam throws {@link KxUnimplemented}.
   */
  async submitWorkflow(
    request: MessageInitShape<typeof SubmitWorkflowRequestSchema>,
    opts: { wait?: boolean; timeoutMs?: number } = {},
  ): Promise<{ instanceId: Uint8Array; recipeFingerprint: Uint8Array } | Result> {
    const handle = await rpc(this.grpc.submitWorkflow(request));
    if (!opts.wait) return handle;
    const outcome = await pollAny(this.grpc, handle.instanceId, opts.timeoutMs ?? 120_000);
    return this._finish(outcome);
  }

  /**
   * Lower a {@link Chain} (the Chains DSL) to a `SubmitWorkflow` request and run it
   * — a thin sugar over {@link KxClientBase.submitWorkflow} (`runChain(c) ==
   * submitWorkflow(c.build())`). The server still COMPILES the lowered DAG, derives
   * all identity, and builds every warrant from the party's grants (SN-8); the chain
   * only changes what is PROPOSED. Returns the run handle, or — with `wait: true` —
   * the first committed {@link Result}. An old gateway without the workflow seam
   * throws {@link KxUnimplemented}.
   */
  async runChain(
    chain: Chain,
    opts: { wait?: boolean; timeoutMs?: number } = {},
  ): Promise<{ instanceId: Uint8Array; recipeFingerprint: Uint8Array } | Result> {
    return this.submitWorkflow(chain.build(), opts);
  }

  async getProjection(instanceId: Id, opts: { atSeq?: bigint } = {}): Promise<Projection> {
    const inst = asBytes(instanceId, INSTANCE_LEN);
    const view = await rpc(this.grpc.getProjection({ instanceId: inst, atSeq: opts.atSeq }));
    return Projection.fromProto(view);
  }

  /**
   * Fetch content by ref. With an `instanceId` (the run ownership ticket) it
   * reads the run scope; OMITTED, it reads the UPLOADS scope (refs this party
   * uploaded via {@link putContent}) — Batch A. Denials are uniform (no
   * existence oracle).
   */
  async getContent(ref: Id, instanceId?: Id): Promise<Uint8Array> {
    const cref = asBytes(ref, REF_LEN);
    const inst = instanceId === undefined ? new Uint8Array() : asBytes(instanceId, INSTANCE_LEN);
    const blob = await rpc(this.grpc.getContent({ contentRef: cref, instanceId: inst }));
    return blob.payload;
  }

  /**
   * Upload bytes to the gateway's content store (Batch A). A CONTENT-STORE
   * write, never a journal write: the returned ref is SERVER-DERIVED blake3
   * (SN-8). `mediaType`/`filename` are advisory audit fields. The server caps
   * the payload fail-closed (`kx serve --content-max-bytes`, default 32 MiB).
   * An old gateway without this RPC throws {@link KxUnimplemented}.
   */
  async putContent(
    payload: Uint8Array,
    opts: { mediaType?: string; filename?: string } = {},
  ): Promise<PutResult> {
    const resp = await rpc(
      this.grpc.putContent({
        payload,
        mediaType: opts.mediaType ?? "",
        filename: opts.filename ?? "",
      }),
    );
    return PutResult.fromProto(resp);
  }

  /**
   * Fetch up to 64 refs in ONE round trip (Batch A — the N+1 collapse), in
   * request order. `instanceId` scopes to a run; omitted reads the uploads
   * scope. Unauthorized/missing/malformed refs come back as UNIFORM empty
   * items ({@link ContentItem.missing}) — no existence oracle. Payloads
   * truncate at `min(maxBytesPerItem, the server's per-item clamp)` with
   * `truncated` set and `fullSize` honest. More than 64 refs is refused.
   */
  async getContentBatch(
    refs: readonly Id[],
    opts: { instanceId?: Id; maxBytesPerItem?: bigint } = {},
  ): Promise<ContentItem[]> {
    const contentRefs = refs.map((r) => asBytes(r, REF_LEN));
    const instanceId =
      opts.instanceId === undefined ? new Uint8Array() : asBytes(opts.instanceId, INSTANCE_LEN);
    const resp = await rpc(
      this.grpc.getContentBatch({
        instanceId,
        contentRefs,
        maxBytesPerItem: opts.maxBytesPerItem,
      }),
    );
    return resp.items.map((i) => ContentItem.fromProto(i));
  }

  /**
   * Discover the models the connected gateway serves (Batch A). Display only
   * (SN-8): selection stays a recipe ENUM free-param validated server-side.
   * An FFI-free gateway returns an EMPTY list; an old gateway without this
   * RPC throws {@link KxUnimplemented}.
   */
  async listModels(): Promise<ModelSummary[]> {
    const resp = await rpc(this.grpc.listModels({}));
    return resp.models.map((m) => ModelSummary.fromProto(m));
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

  /**
   * The operator-global cross-run event tail (Batch C `StreamAllEvents`): every
   * run's deltas plus `run_registered` narration, watermark-attributed by
   * `instanceId` (`""` before any registration). Same cursor contract as
   * {@link streamEvents} (resumes from `next_seq` on a slow-consumer drop when
   * `follow`). An old gateway without this RPC throws {@link KxUnimplemented}.
   */
  streamAllEvents(
    opts: { since?: bigint; follow?: boolean; signal?: AbortSignal } = {},
  ): AsyncIterable<GlobalDelta> {
    return streamAllDeltas(this.grpc, opts.since ?? 0n, opts.follow ?? false, opts.signal);
  }

  /** Consume the operator-global live tail over the WS bridge (Batch C
   *  `/v1/events/all` — browser-friendly, same bearer auth as {@link wsEvents}). */
  async *wsAllEvents(
    opts: { since?: bigint; wsEndpoint?: string } = {},
  ): AsyncIterable<GlobalDelta> {
    const url = wsAllUrl(this.endpoint, opts.wsEndpoint ?? this.wsEndpoint, opts.since ?? 0n);
    try {
      yield* wsAllDeltasFromMessages(this.openWsMessages(url, this.token));
    } catch (e) {
      // An OLD bridge rejects the global handshake with HTTP 400 (the channel
      // didn't exist) — surface the SDK's not-wired signal, not a raw socket
      // error. (Node `ws` reports the status; a browser error event carries
      // none, so there it stays a KxConnectError.)
      if (e instanceof KxConnectError && e.message.includes("Unexpected server response: 400")) {
        throw new KxUnimplemented(
          "the gateway's WS bridge has no /v1/events/all channel (old server)",
        );
      }
      throw e;
    }
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

  /**
   * Enumerate the durable registered runs (newest-first, paginated) — the
   * "re-open by instance-id" primitive. `beforeSeq` resumes from the
   * `registeredSeq` of the last run seen; `limit` bounds the page (server-clamped).
   * An old gateway without this RPC throws {@link KxUnimplemented}.
   */
  async listRuns(opts: { limit?: number; beforeSeq?: bigint } = {}): Promise<RunPage> {
    const resp = await rpc(this.grpc.listRuns({ limit: opts.limit, beforeSeq: opts.beforeSeq }));
    return { runs: resp.runs.map((r) => RunSummary.fromProto(r)), hasMore: resp.hasMore };
  }

  /**
   * Resolve one Mote's admitted definition (Batch B) — the node-inspector
   * read: step kind, model, prompt, capped params, tool contract. DISPLAY
   * ONLY (SN-8). The detail is commit-gated: an uncommitted mote (or one
   * admitted by a pre-Batch-B binary) answers `defFound: false` honestly; an
   * unknown mote in an owned run throws {@link KxNotFound}; a wrong ticket
   * throws the uniform {@link KxPermissionDenied}. An old gateway without
   * this RPC throws {@link KxUnimplemented}.
   */
  async getMoteDetail(instanceId: Id, moteId: Id): Promise<MoteDetail> {
    const inst = asBytes(instanceId, INSTANCE_LEN);
    const mote = asBytes(moteId, REF_LEN);
    const resp = await rpc(this.grpc.getMoteDetail({ instanceId: inst, moteId: mote }));
    return MoteDetail.fromProto(resp);
  }

  /**
   * Enumerate a live ReAct chain's durable turn facts (newest-first, paginated)
   * — the queryable Reason→Act→Observe history (PR-2d-1/2). `instanceId` (hex)
   * scopes to one run; absent enumerates every chain. The server clamps `limit`
   * to its max page. An old gateway without this RPC throws {@link KxUnimplemented}.
   */
  async listReactTurns(opts: { instanceId?: string; limit?: number } = {}): Promise<ReactTurnPage> {
    const instanceId =
      opts.instanceId === undefined ? undefined : asBytes(opts.instanceId, INSTANCE_LEN);
    const resp = await rpc(this.grpc.listReactTurns({ instanceId, limit: opts.limit }));
    return { turns: resp.turns.map((t) => ReactTurn.fromProto(t)), hasMore: resp.hasMore };
  }

  /**
   * Enumerate a run's model-driven re-plan rounds (newest-first, paginated;
   * PR-2c-2). The server clamps `limit` to its max page. An old gateway without
   * this RPC throws {@link KxUnimplemented}.
   */
  async listReplanRounds(opts: { limit?: number } = {}): Promise<ReplanRoundPage> {
    const resp = await rpc(this.grpc.listReplanRounds({ limit: opts.limit }));
    return { rounds: resp.rounds.map((r) => ReplanRound.fromProto(r)), hasMore: resp.hasMore };
  }

  /**
   * Enumerate the Morphic Data Engine's durably-captured ACTION records
   * (newest-first, paginated) — the serve-path action exhaust. `instanceId`
   * (hex) scopes to one run; absent enumerates every captured action. The
   * server clamps `limit` to its max page. An old gateway (or one without the
   * capture sidecar) throws {@link KxUnimplemented}.
   */
  async listCaptureRecords(
    opts: { instanceId?: string; limit?: number } = {},
  ): Promise<CaptureRecordPage> {
    const instanceId =
      opts.instanceId === undefined ? undefined : asBytes(opts.instanceId, INSTANCE_LEN);
    const resp = await rpc(this.grpc.listCaptureRecords({ instanceId, limit: opts.limit }));
    return { records: resp.records.map((r) => CaptureRecord.fromProto(r)), hasMore: resp.hasMore };
  }

  /**
   * Enumerate the host-measured mote execution telemetry (newest-first,
   * paginated; Batch C) — wall-clock, model usage, the fired tool, from the
   * gateway's rebuildable-to-empty `telemetry.db` sidecar (audit/display only,
   * never truth). `instanceId` (hex) scopes to one run, `moteId` (hex) to one
   * mote; `beforeSeq` resumes from the `seq` of the last row seen. The server
   * clamps `limit` to its max page. An old gateway (or one without the sidecar)
   * throws {@link KxUnimplemented}.
   */
  async listMoteTelemetry(
    opts: { instanceId?: string; moteId?: string; limit?: number; beforeSeq?: bigint } = {},
  ): Promise<MoteTelemetryPage> {
    const instanceId =
      opts.instanceId === undefined ? undefined : asBytes(opts.instanceId, INSTANCE_LEN);
    const moteId = opts.moteId === undefined ? undefined : asBytes(opts.moteId, REF_LEN);
    const resp = await rpc(
      this.grpc.listMoteTelemetry({
        instanceId,
        moteId,
        limit: opts.limit,
        beforeSeq: opts.beforeSeq,
      }),
    );
    return { rows: resp.rows.map((r) => MoteTelemetryRow.fromProto(r)), hasMore: resp.hasMore };
  }

  /**
   * Record 👍/👎 feedback on an answer (PR-4.1) — a client-origin write into the
   * gateway's rebuildable-to-empty `feedback.db` sidecar (advisory product
   * signal, never truth/identity/a digest input). The caller principal + the
   * returned `feedbackId` (hex) are SERVER-derived; re-rating the same answer
   * OVERWRITES. `messageId` is the stable per-answer key; the rest are advisory
   * join/context. An old gateway without this RPC throws {@link KxUnimplemented}.
   */
  async submitFeedback(input: FeedbackInput): Promise<string> {
    const resp = await rpc(
      this.grpc.submitFeedback({
        rating: ratingToProto(input.rating),
        messageId: input.messageId,
        instanceId:
          input.instanceId === undefined ? undefined : asBytes(input.instanceId, INSTANCE_LEN),
        moteId: input.moteId === undefined ? undefined : asBytes(input.moteId, REF_LEN),
        contentRef: input.contentRef === undefined ? undefined : asBytes(input.contentRef, REF_LEN),
        comment: input.comment ?? "",
        recipeHandle: input.recipeHandle ?? "",
        modelId: input.modelId ?? "",
      }),
    );
    return encode(resp.feedbackId);
  }

  /**
   * Read back recorded feedback (newest-first, paginated; PR-4.1) from the
   * gateway's `feedback.db` sidecar — audit/inspection only. `instanceId` (hex)
   * scopes to one run; `beforeRowid` resumes from the last row seen. The server
   * clamps `limit` to its max page. An old gateway (or one without the sidecar)
   * throws {@link KxUnimplemented}.
   */
  async listFeedback(
    opts: { instanceId?: string; limit?: number; beforeRowid?: bigint } = {},
  ): Promise<FeedbackPage> {
    const instanceId =
      opts.instanceId === undefined ? undefined : asBytes(opts.instanceId, INSTANCE_LEN);
    const resp = await rpc(
      this.grpc.listFeedback({ instanceId, limit: opts.limit, beforeRowid: opts.beforeRowid }),
    );
    return { rows: resp.rows.map((r) => FeedbackRow.fromProto(r)), hasMore: resp.hasMore };
  }

  /**
   * List the invocable recipe handles the gateway provisions (the public recipe
   * catalog). An old gateway without this RPC throws {@link KxUnimplemented}.
   */
  async listRecipes(): Promise<string[]> {
    const resp = await rpc(this.grpc.listRecipes({}));
    return resp.recipes.map((r) => r.handle);
  }

  /**
   * The recipe catalog WITH each recipe's published workflow fingerprint
   * (PR-2.1) — the join key for labeling durable {@link RunSummary} rows by
   * recipe handle. `recipeFingerprint` is `""` on a gateway predating the
   * field (additive — degrade to unlabeled rows).
   */
  async listRecipeSummaries(): Promise<RecipeInfo[]> {
    const resp = await rpc(this.grpc.listRecipes({}));
    return resp.recipes.map((r) => RecipeInfo.fromProto(r));
  }

  /**
   * ADVISORY recipe discovery (PR-4 Batch D) — rank the provisioned recipes
   * against `intent` (+ optional `keywords`), best-first, capped at `limit`.
   * SN-8: each `scoreBp` is DISPLAY-ONLY (a hit SURFACES a recipe, never invokes
   * one — {@link KxClientBase.invoke} stays the authorization gate). An old
   * gateway / a catalog with no ranker throws {@link KxUnimplemented}.
   */
  async searchRecipes(
    intent: string,
    opts: { keywords?: readonly string[]; limit?: number } = {},
  ): Promise<ScoredRecipe[]> {
    const resp = await rpc(
      this.grpc.searchRecipes({
        intent,
        keywords: opts.keywords ? [...opts.keywords] : [],
        limit: opts.limit,
      }),
    );
    return resp.ranked.map((s) => ScoredRecipe.fromProto(s));
  }

  /**
   * The free-param {@link RecipeForm} for `handle` (render an input form, then
   * {@link KxClientBase.invoke}). An unknown handle throws {@link KxNotFound}; an
   * old gateway without this RPC throws {@link KxUnimplemented}.
   */
  async getRecipeForm(handle: string): Promise<RecipeForm> {
    const resp = await rpc(this.grpc.getRecipeForm({ handle }));
    return RecipeForm.fromProto(resp);
  }

  /**
   * Enumerate the teams the gateway knows (UI-3 Systems viewer). VIEW-only in OSS.
   * An old gateway without this RPC throws {@link KxUnimplemented}.
   */
  async listTeams(): Promise<TeamSummary[]> {
    const resp = await rpc(this.grpc.listTeams({}));
    return teamsFromProto(resp);
  }

  /**
   * The members of `teamId` (+ each member's role/caps). When `assetRef` is given,
   * each member's resolved warrant on that asset (membership ∩ grant, ⊆ the team) is
   * populated. An unknown team throws {@link KxNotFound}; an old gateway without this
   * RPC throws {@link KxUnimplemented}.
   */
  async listTeamMembers(teamId: string, opts: { assetRef?: string } = {}): Promise<TeamMembers> {
    const resp = await rpc(this.grpc.listTeamMembers({ teamId, assetRef: opts.assetRef }));
    return TeamMembers.fromProto(resp);
  }

  /**
   * Every grant on `assetRef`, fold-classified root/delegated + active/revoked (the
   * UI-3 sharing inspector). An unknown asset throws {@link KxNotFound}; an old
   * gateway without this RPC throws {@link KxUnimplemented}.
   */
  async listAssetGrants(assetRef: string): Promise<AssetGrants> {
    const resp = await rpc(this.grpc.listAssetGrants({ assetRef }));
    return AssetGrants.fromProto(resp);
  }

  /**
   * Every dataset (RAG corpus) on the gateway (T3.7). An old gateway / a build
   * without the `hnsw` feature throws {@link KxUnimplemented}.
   */
  async listDatasets(): Promise<DatasetSummary[]> {
    const resp = await rpc(this.grpc.listDatasets({}));
    return resp.datasets.map((d) => DatasetSummary.fromProto(d));
  }

  /**
   * Ingest `documents` into `dataset` (created on first ingest). Each doc carries
   * `content` (always) + an OPTIONAL client-computed `embedding` (the FFI-free
   * path); a vector-less doc needs a gateway with the `inference` feature (else
   * {@link KxFailedPrecondition}). The server derives each doc's id from its content
   * (SN-8); re-ingesting identical content is a no-op (content-addressed dedup).
   */
  async ingestDocuments(dataset: string, documents: readonly IngestDoc[]): Promise<IngestResult> {
    const resp = await rpc(
      this.grpc.ingestDocuments({
        dataset,
        documents: documents.map((d) => ({
          content: d.content,
          embedding: d.embedding ? Array.from(d.embedding) : [],
          docId: d.docId,
          metadata: d.metadata ? { ...d.metadata } : {},
        })),
      }),
    );
    return IngestResult.fromProto(resp);
  }

  /**
   * Query `dataset` for the top-`k` nearest documents. Pass `embedding` (the
   * FFI-free client-vector path, takes precedence) or `text` (server-embed, needs
   * the `inference` feature). Hits are ordered by the DISPLAY-ONLY score (SN-8). An
   * unknown dataset throws {@link KxNotFound}.
   */
  async queryDataset(
    dataset: string,
    opts: { text?: string; embedding?: readonly number[]; k?: number } = {},
  ): Promise<DatasetHit[]> {
    const resp = await rpc(
      this.grpc.queryDataset({
        dataset,
        queryText: opts.text ?? "",
        queryEmbedding: opts.embedding ? Array.from(opts.embedding) : [],
        k: opts.k ?? 10,
      }),
    );
    return resp.hits.map((h) => DatasetHit.fromProto(h));
  }

  /**
   * Enumerate the registered tools' advisory manifests (W1.A5; deterministic
   * (toolId, toolVersion) order). DISPLAY-ONLY (SN-8): manifests rank/describe,
   * never authorize — the broker never reads them. An old gateway without this
   * RPC throws {@link KxUnimplemented}.
   */
  async listToolManifests(): Promise<ToolManifest[]> {
    const resp = await rpc(this.grpc.listToolManifests({}));
    return resp.manifests.map((m) => ToolManifest.fromProto(m));
  }

  /**
   * Score a client-authored TaskBundle `spec` against every registered manifest
   * (W1.A5): advisory basis-point ranks + a server-side DRY-RUN of the real
   * lowering gate (the SERVER-built warrant — no client warrant input; nothing
   * submits, nothing journals). ADVISORY/DISPLAY-ONLY (SN-8): a score can surface
   * a tool, never grant one. An invalid spec throws {@link KxInvalidArgument}; an
   * old gateway without this RPC throws {@link KxUnimplemented}.
   */
  async scoreTaskBundle(spec: BundleSpec): Promise<BundleScore> {
    const resp = await rpc(this.grpc.scoreTaskBundle(bundleSpecToProto(spec)));
    return BundleScore.fromProto(resp);
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
