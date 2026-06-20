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
import { AlertSummary, type AlertsPage } from "./alerts.js";
import { AdvanceResult, Branch, CreateBranchResult, SnapshotResult } from "./branch.js";
import { CaptureRecord, type CaptureRecordPage } from "./capture.js";
import type { Chain } from "./chains.js";
import { ContentItem, PutResult } from "./content.js";
import { ContextBundle, type ContextItemInput, PutContextBundleResult } from "./context.js";
import { DatasetHit, DatasetSummary, type IngestDoc, IngestResult } from "./datasets.js";
import { KxConnectError, KxRunFailed, KxUnimplemented, KxWaitTimeout, rpc } from "./errors.js";
import {
  streamAllDeltas,
  streamDeltas,
  streamModelTokens,
  wsAllDeltasFromMessages,
  wsAllUrl,
  wsDeltasFromMessages,
  wsTokenChunksFromMessages,
  wsTokenUrl,
  wsUrl,
} from "./events.js";
import { type FeedbackInput, type FeedbackPage, FeedbackRow, ratingToProto } from "./feedback.js";
import { FuzzyHit } from "./fuzzy.js";
import {
  KxGateway,
  type SubmitRunRequestSchema,
  type SubmitWorkflowRequestSchema,
  WorkflowStepKind,
} from "./gen/kortecx/v1/gateway_pb.js";
import { AssetGrants } from "./grants.js";
import { INSTANCE_LEN, REF_LEN, asBytes, encode } from "./hexids.js";
import { ModelSummary } from "./models.js";
import { MoteDetail } from "./motes.js";
import { ReactTurn, type ReactTurnPage } from "./react.js";
import { RecipeForm, RecipeInfo, ScoredRecipe } from "./recipes.js";
import { ReplanRound, type ReplanRoundPage } from "./replan.js";
import { Result, Run } from "./run.js";
import { RunInputs, type RunPage, RunSummary } from "./runs.js";
import { TeamMembers, type TeamSummary, teamsFromProto } from "./teams.js";
import { type MoteTelemetryPage, MoteTelemetryRow, TelemetrySummary } from "./telemetry.js";
import type { TokenChunk } from "./tokens.js";
import {
  BundleScore,
  type BundleSpec,
  McpServer,
  type McpServersPage,
  type RegisterMcpServerInput,
  type RegisterServerResult,
  type RegisterToolInput,
  RegisteredTool,
  type RegisteredToolsPage,
  ToolManifest,
  bundleSpecToProto,
} from "./toolscout.js";
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
  /**
   * Batch A: the default model to fill into MODEL steps that omit `modelId` (a
   * multi-model convenience; the server binds `""` → served when unset). On Node an
   * explicit value wins over the `KX_DEFAULT_MODEL` env fallback.
   */
  defaultModel?: string;
}

/**
 * Batch A: fill any MODEL step that left `modelId` empty with `defaultModel`, in
 * place, just before submit. A no-op when `defaultModel` is unset OR no step omitted
 * its model — so the canonical lowering (corpus-pinned, client-free) is untouched and
 * the server still binds `""` → served (SN-8) when neither is set.
 */
export function fillDefaultModel(
  request: MessageInitShape<typeof SubmitWorkflowRequestSchema>,
  defaultModel: string,
): void {
  if (!defaultModel || !request.steps) return;
  for (const step of request.steps) {
    if (step.kind === WorkflowStepKind.MODEL && !step.modelId) {
      step.modelId = defaultModel;
    }
  }
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
  /**
   * PR-7: context-bundle handles to attach. The server resolves each to its item
   * refs and injects them into the entry Mote's IDENTITY-BEARING context, so a
   * different context ⇒ a different run.
   */
  context?: readonly string[];
  /**
   * D155 Phase-3: raw 64-hex content-store refs to attach directly as context
   * (no bundle needed). Same identity-bearing injection as {@link context}.
   */
  contextRefs?: readonly string[];
}

export abstract class KxClientBase {
  readonly endpoint: string;
  protected readonly token: string | undefined;
  protected readonly wsEndpoint: string | undefined;
  /** Batch A: the default model filled into MODEL steps that omit `modelId`. */
  readonly defaultModel: string;
  protected readonly grpc: Client<typeof KxGateway>;

  protected constructor(
    endpoint: string,
    transport: Transport,
    opts: { token?: string; wsEndpoint?: string; defaultModel?: string },
  ) {
    this.endpoint = endpoint;
    this.token = opts.token;
    this.wsEndpoint = opts.wsEndpoint;
    this.defaultModel = opts.defaultModel ?? "";
    this.grpc = createClient(KxGateway, transport);
  }

  /**
   * Bind a published recipe to `args` and run it. With `wait: true` blocks for the
   * committed {@link Result} (throwing {@link KxRunFailed} / {@link KxWaitTimeout}
   * on a failed / timed-out run); otherwise returns a {@link Run} handle.
   */
  async invoke(handle: string, args: Args, opts: InvokeOptions = {}): Promise<Run | Result> {
    const argBytes = encodeArgs(args);
    const resp = await rpc(
      this.grpc.invoke({
        handle,
        args: argBytes,
        contextBundles: opts.context ? [...opts.context] : [],
        contextRefs: opts.contextRefs ? [...opts.contextRefs] : [],
      }),
    );
    const run = new Run(this, resp.instanceId, resp.terminalMoteId, resp.recipeFingerprint);
    if (!opts.wait) return run;
    const result =
      // React CHAIN recipes (react / react-fs / react-auto) settle via
      // ListReactTurns, not a terminal Mote (F13); they share the prefix.
      // react-edit is EXCLUDED — a single model step settling on its terminal mote.
      handle.startsWith(REACT_RECIPE_HANDLE) && handle !== "kx/recipes/react-edit"
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
    fillDefaultModel(request, this.defaultModel);
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
    // V2b: register + resolve any `localTool(...)` functions the chain references
    // (a chain with none is unaffected — `resolved` is undefined ⇒ build() byte-identical).
    const { resolveLocalTools } = await import("./tools.js");
    const resolved = await resolveLocalTools(this, chain);
    return this.submitWorkflow(chain.build(resolved), opts);
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
   * Author (upsert) a context bundle (PR-7) at `handle` for this party. Each item
   * names a `contentRef` already in the content store (e.g. from
   * {@link putContent}). The server derives `bundleRef` (SN-8) into an off-journal
   * sidecar. Attach the handle to a run with `invoke(handle, args, { context: [h] })`.
   */
  async putContextBundle(
    handle: string,
    items: readonly ContextItemInput[],
    opts: { description?: string } = {},
  ): Promise<PutContextBundleResult> {
    const protoItems = items.map((it) => ({
      name: it.name,
      contentRef: asBytes(it.contentRef, REF_LEN),
      mediaType: it.mediaType ?? "",
    }));
    const resp = await rpc(
      this.grpc.putContextBundle({
        handle,
        description: opts.description ?? "",
        items: protoItems,
      }),
    );
    return PutContextBundleResult.fromProto(resp);
  }

  /** List this party's context bundles (PR-7) in handle order. */
  async listContextBundles(): Promise<ContextBundle[]> {
    const resp = await rpc(this.grpc.listContextBundles({}));
    return resp.bundles.map((b) => ContextBundle.fromProto(b));
  }

  /**
   * Fetch one context bundle by handle, or `null` if not found / not owned
   * (uniform — no cross-party existence oracle).
   */
  async getContextBundle(handle: string): Promise<ContextBundle | null> {
    const resp = await rpc(this.grpc.getContextBundle({ handle }));
    return resp.found && resp.bundle ? ContextBundle.fromProto(resp.bundle) : null;
  }

  /** Unbind a context bundle (its CAS blobs stay). Returns `true` iff removed. */
  async deleteContextBundle(handle: string): Promise<boolean> {
    const resp = await rpc(this.grpc.deleteContextBundle({ handle }));
    return resp.removed;
  }

  /**
   * Create (or fork via `opts.parent`) a D155 branch at `handle` for this party.
   * A `parent` handle forks a point-in-time CoW sub-branch (it inherits the
   * parent's resolved items at create time; later parent edits do not propagate).
   * The server derives `branchRef` (SN-8) into an off-journal sidecar.
   */
  async createBranch(
    handle: string,
    opts: { parent?: string; description?: string } = {},
  ): Promise<CreateBranchResult> {
    const resp = await rpc(
      this.grpc.createBranch({
        handle,
        description: opts.description ?? "",
        parentHandle: opts.parent ?? "",
      }),
    );
    return CreateBranchResult.fromProto(resp);
  }

  /**
   * Snapshot operator-approved host `paths` into the branch `handle` (created if
   * absent, optionally from `opts.parent`). Each path is read (confined under
   * `KX_SERVE_FS_ROOT`, default-OFF) INTO the content store; the `{path -> ref}`
   * manifest is recorded/merged. The host is never written (Phase-A). Rejects with
   * a FAILED_PRECONDITION when `KX_SERVE_FS_ROOT` is unset.
   */
  async snapshotInto(
    handle: string,
    paths: readonly string[],
    opts: { parent?: string; description?: string } = {},
  ): Promise<SnapshotResult> {
    const resp = await rpc(
      this.grpc.snapshotInto({
        handle,
        paths: [...paths],
        description: opts.description ?? "",
        parentHandle: opts.parent ?? "",
      }),
    );
    return SnapshotResult.fromProto(resp);
  }

  /** List this party's D155 branches in handle order. */
  async listBranches(): Promise<Branch[]> {
    const resp = await rpc(this.grpc.listBranches({}));
    return resp.branches.map((b) => Branch.fromProto(b));
  }

  /**
   * Fetch one branch's resolved manifest by handle, or `null` if not found / not
   * owned (uniform — no cross-party existence oracle).
   */
  async getBranch(handle: string): Promise<Branch | null> {
    const resp = await rpc(this.grpc.getBranch({ handle }));
    return resp.found && resp.branch ? Branch.fromProto(resp.branch) : null;
  }

  /** Unbind a branch (its CAS blobs stay). Returns `true` iff removed. */
  async deleteBranch(handle: string): Promise<boolean> {
    const resp = await rpc(this.grpc.deleteBranch({ handle }));
    return resp.removed;
  }

  /**
   * D155 Phase-3 (low-level): re-point `path` in branch `handle` to an EXISTING
   * content-store ref `contentRef` (64-hex), or insert it ("enrich"), then
   * recompute `branchRef`. Strictly IN-CAS (no host read/write). Rejects
   * NOT_FOUND if the branch is unknown, INVALID_ARGUMENT if the ref does not
   * resolve. Prefer {@link editBranch} for the agentic high-level flow.
   */
  async advanceBranch(handle: string, path: string, contentRef: string): Promise<AdvanceResult> {
    const resp = await rpc(
      this.grpc.advanceBranch({ handle, path, contentRef: asBytes(contentRef, REF_LEN) }),
    );
    return AdvanceResult.fromProto(resp);
  }

  /**
   * D155 Phase-3: agentically edit a branch file IN-CAS. Resolves `path`'s current
   * ref, runs the `kx/recipes/react-edit` loop (the body attached as a context
   * ref; the model rewrites it per `instruction`), and advances the manifest to
   * the new content ref. The host is NEVER written. Rejects if the chain produced
   * no committed answer.
   */
  async editBranch(
    handle: string,
    path: string,
    instruction: string,
    opts: { timeoutMs?: number } = {},
  ): Promise<AdvanceResult> {
    const branch = await this.getBranch(handle);
    if (branch === null) throw new Error(`branch '${handle}' not found`);
    const item = branch.items.find((it) => it.path === path);
    if (item === undefined) throw new Error(`path '${path}' is not in branch '${handle}'`);
    const directive = `You are editing the file \`${path}\`. The text in the attached context below IS its exact current contents. Apply this change: ${instruction}\n\nReturn ONLY the complete, updated file contents — no commentary, no explanation, and no markdown code fences.`;
    // react-edit is a single model step; its only free param is `prompt`.
    const result = await this.invoke(
      "kx/recipes/react-edit",
      { prompt: directive },
      { wait: true, timeoutMs: opts.timeoutMs ?? 300_000, contextRefs: [item.contentRef] },
    );
    if (!(result instanceof Result) || !result.ok || result.resultRef === null) {
      throw new Error("react-edit produced no committed answer to advance the branch to");
    }
    // Fail CLOSED on an empty edit (GR15): never advance the manifest to an empty
    // file (a heavy-reasoning model can return only stripped reasoning).
    if (result.payload === null || result.payload.length === 0) {
      throw new Error(
        "react-edit produced an empty body (the model did not return file contents); the branch was NOT advanced",
      );
    }
    return this.advanceBranch(handle, path, result.resultRef);
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
   * Native gRPC server-streaming ADVISORY token tail for ONE model mote (PR-4.2 /
   * T-STREAM1): the NEW bytes per decode step until `done`. `moteId` must belong
   * to `instanceId`'s run (server-gated). The committed `result_ref` stays the
   * authority — reconcile to it. An old gateway throws {@link KxUnimplemented}.
   */
  streamModelTokens(
    instanceId: Id,
    moteId: Id,
    opts: { since?: bigint; signal?: AbortSignal } = {},
  ): AsyncIterable<TokenChunk> {
    const inst = asBytes(instanceId, INSTANCE_LEN);
    const mote = asBytes(moteId, REF_LEN);
    return streamModelTokens(this.grpc, inst, mote, opts.since ?? 0n, opts.signal);
  }

  /**
   * Consume ONE model mote's ADVISORY token stream over the WS bridge (PR-4.2 —
   * the browser's only live token path; a browser cannot speak gRPC server-
   * streaming). Same bearer auth as {@link wsEvents}.
   */
  async *wsTokens(
    instanceId: Id,
    moteId: Id,
    opts: { since?: bigint; wsEndpoint?: string } = {},
  ): AsyncIterable<TokenChunk> {
    const inst = asBytes(instanceId, INSTANCE_LEN);
    const mote = asBytes(moteId, REF_LEN);
    const url = wsTokenUrl(
      this.endpoint,
      opts.wsEndpoint ?? this.wsEndpoint,
      encode(inst),
      encode(mote),
      opts.since ?? 0n,
    );
    yield* wsTokenChunksFromMessages(this.openWsMessages(url, this.token));
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
   * The args a run was submitted with (PR-D `GetRunInputs`) — the baseline for
   * "Re-run with changes": fetch the captured args + handle, edit, then
   * {@link invoke} again (only the changed sub-DAG recomputes). Useful when a run
   * is recovered from {@link listRuns} with no client-side state. A run with
   * nothing captured throws {@link KxNotFound}; an old gateway without the sidecar
   * throws {@link KxUnimplemented}.
   */
  async getRunInputs(instanceId: Id): Promise<RunInputs> {
    const inst = asBytes(instanceId, INSTANCE_LEN);
    const resp = await rpc(this.grpc.getRunInputs({ instanceId: inst }));
    return RunInputs.fromProto(resp);
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
   * The EXACT, cross-page per-model token-economy rollup (W1a-3) — output tokens
   * + wall-clock summed `GROUP BY model_id` server-side over the same
   * `telemetry.db` sidecar, so a long ReAct run is summed honestly (unlike a
   * client fold over the page-clamped {@link listMoteTelemetry}). Token-only, no
   * cost/$ (billing is CLOUD). `instanceId` (hex) scopes to one run; absent sums
   * all runs. An old gateway (or one without the sidecar) throws
   * {@link KxUnimplemented}.
   */
  async listTelemetrySummary(opts: { instanceId?: string } = {}): Promise<TelemetrySummary> {
    const instanceId =
      opts.instanceId === undefined ? undefined : asBytes(opts.instanceId, INSTANCE_LEN);
    const resp = await rpc(this.grpc.listTelemetrySummary({ instanceId }));
    return TelemetrySummary.fromProto(resp);
  }

  /**
   * Enumerate the operator alerts inbox (newest-first, paginated) — the
   * journal's TERMINAL `Failed` facts (dead-letters + worker-reported terminal
   * failures) folded into the gateway's rebuildable-to-empty `alerts.db`
   * read-cache (W1a-2). DISPLAY/TRIAGE-READ only: never truth, never identity,
   * never a digest input. `instanceId` (hex) scopes to one run; `beforeSeq`
   * resumes from the `seq` of the last row seen. The server clamps `limit` to its
   * max page. The triage lifecycle (ack/resolve) is a Cloud capability (D156) —
   * not exposed here. An old gateway (or one without the sidecar) throws
   * {@link KxUnimplemented}.
   */
  async listAlerts(
    opts: { instanceId?: string; limit?: number; beforeSeq?: bigint } = {},
  ): Promise<AlertsPage> {
    const instanceId =
      opts.instanceId === undefined ? undefined : asBytes(opts.instanceId, INSTANCE_LEN);
    const resp = await rpc(
      this.grpc.listAlerts({
        instanceId,
        limit: opts.limit,
        beforeSeq: opts.beforeSeq,
      }),
    );
    return { alerts: resp.alerts.map((a) => AlertSummary.fromProto(a)), hasMore: resp.hasMore };
  }

  /**
   * The durable tools registry INVENTORY (PR-6a `DiscoverTools`) — registered
   * tools + their authority/provenance, in `(name, version)` order. DISTINCT from
   * {@link KxClientBase.listToolManifests} (advisory ranking). Registration grants
   * NO authority (SN-8). An old gateway (or one without the registry) throws
   * {@link KxUnimplemented}.
   */
  async discoverTools(
    opts: { limit?: number; afterName?: string; afterVersion?: string } = {},
  ): Promise<RegisteredToolsPage> {
    const resp = await rpc(
      this.grpc.discoverTools({
        limit: opts.limit ?? 0,
        afterName: opts.afterName ?? "",
        afterVersion: opts.afterVersion ?? "",
      }),
    );
    return { tools: resp.tools.map((t) => RegisteredTool.fromProto(t)), hasMore: resp.hasMore };
  }

  /**
   * Register a declarative EXTERNAL MCP tool (PR-6a `RegisterTool`). The server
   * SSRF-vets `serverHost`, derives identity + capability, and durably stores it;
   * the returned `toolId` (hex) is SERVER-derived (the client never names/forges
   * it, SN-8). Registration grants NO authority — a tool fires only under a
   * server-issued warrant. DIALING `serverHost` is a Cloud / PR-6b capability. An
   * internal/link-local host is refused (`permission_denied`).
   */
  async registerTool(input: RegisterToolInput): Promise<string> {
    const inputSchema =
      input.params && input.params.length > 0
        ? {
            params: input.params.map((p) => ({
              name: p.name,
              ty: p.ty ?? "str",
              maxLen: p.maxLen ?? 0,
              required: p.required ?? true,
              allowed: [...(p.allowed ?? [])],
            })),
            denyUnknown: input.denyUnknownParams ?? true,
          }
        : undefined;
    const resp = await rpc(
      this.grpc.registerTool({
        toolName: input.name,
        toolVersion: input.version,
        description: input.description ?? "",
        idempotencyClass: input.idempotencyClass ?? "Readback",
        inputSchema,
        serverHost: input.serverHost,
        remoteName: input.remoteName ?? "",
      }),
    );
    return encode(resp.toolId);
  }

  /**
   * Deregister an operator-registered tool by exact `(name, version)` (PR-6a
   * `DeregisterTool`). Built-ins are refused (returns `false`). Returns `true` iff
   * a row was removed.
   */
  async deregisterTool(name: string, version: string): Promise<boolean> {
    const resp = await rpc(this.grpc.deregisterTool({ toolName: name, toolVersion: version }));
    return resp.removed;
  }

  // --- PR-6b-1: the external MCP gateway (dial Py/TS-SDK-exposed MCP servers) ---

  /**
   * Register an EXTERNAL MCP server (PR-6b-1 `RegisterMcpServer`) — the runtime
   * DIALS it (`initialize` → `tools/list`) and registers its tools into the
   * durable registry (each namespaced `<name>/<remote>`). The host is SSRF-vetted
   * at admission AND at dial time. `credentialRef` names an env var / vault key
   * (the secret VALUE is never sent, D81). A dial failure is NOT fatal — the
   * server persists with `health="unreachable"` (honest, never a fabricated
   * success). An internal host is refused (`permission_denied`).
   */
  async registerMcpServer(input: RegisterMcpServerInput): Promise<RegisterServerResult> {
    const resp = await rpc(
      this.grpc.registerMcpServer({
        serverName: input.name,
        transport: input.transport ?? "stdio",
        endpoint: input.endpoint,
        args: [...(input.args ?? [])],
        tlsRequired: input.tlsRequired ?? false,
        credentialRef: input.credentialRef ?? "",
        sessionMode: input.sessionMode ?? "stateless",
      }),
    );
    return {
      connectionId: encode(resp.connectionId),
      discovered: resp.discovered,
      health: resp.health,
    };
  }

  /**
   * List the registered external MCP servers + their health (PR-6b-1
   * `ListMcpServers`), in `(name)` order.
   */
  async listMcpServers(opts: { limit?: number; afterName?: string } = {}): Promise<McpServersPage> {
    const resp = await rpc(
      this.grpc.listMcpServers({ limit: opts.limit ?? 0, afterName: opts.afterName ?? "" }),
    );
    return { servers: resp.servers.map((s) => McpServer.fromProto(s)), hasMore: resp.hasMore };
  }

  /**
   * Re-dial a registered server + re-discover its tools (PR-6b-1
   * `DiscoverServerTools`); returns the server's registered tools.
   */
  async discoverServerTools(name: string): Promise<RegisteredToolsPage> {
    const resp = await rpc(this.grpc.discoverServerTools({ serverName: name }));
    return { tools: resp.tools.map((t) => RegisteredTool.fromProto(t)), hasMore: false };
  }

  /**
   * Test a server's reachability — dial + `initialize` only (PR-6b-1
   * `TestMcpServer`). Returns `true` iff the handshake succeeded.
   */
  async testMcpServer(name: string): Promise<boolean> {
    const resp = await rpc(this.grpc.testMcpServer({ serverName: name }));
    return resp.reachable;
  }

  /**
   * Remove a registered server + deregister its tools (PR-6b-1
   * `DeregisterMcpServer`). Returns `true` iff a server was removed.
   */
  async deregisterMcpServer(name: string): Promise<boolean> {
    const resp = await rpc(this.grpc.deregisterMcpServer({ serverName: name }));
    return resp.removed;
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
   * Slice-B advisory fuzzy-in / exact-out discovery over `dataset` (D151). Like
   * {@link queryDataset} (pass `embedding` for the FFI-free client-vector path, or
   * `text` for server-embed), but each {@link FuzzyHit} carries ONLY the
   * content-addressed ref + a DISPLAY-ONLY basis-point score (SN-8) — join back to
   * bytes with an EXACT {@link getContent} on the ref. An old gateway / a build
   * without the `hnsw` feature throws {@link KxUnimplemented}.
   */
  async fuzzyDiscovery(
    dataset: string,
    opts: { text?: string; embedding?: readonly number[]; k?: number } = {},
  ): Promise<FuzzyHit[]> {
    const resp = await rpc(
      this.grpc.fuzzyDiscovery({
        dataset,
        queryText: opts.text ?? "",
        queryEmbedding: opts.embedding ? Array.from(opts.embedding) : [],
        k: opts.k ?? 10,
      }),
    );
    return resp.hits.map((h) => FuzzyHit.fromProto(h));
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
