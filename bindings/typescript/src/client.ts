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
import { AppBundle, MAX_BUNDLE_CLOSURE_BYTES, MAX_BUNDLE_REFS } from "./appbundle.js";
import { PendingApprovalRow, type PendingApprovalsPage } from "./approvals.js";
import {
  AppManifest,
  AppSummary,
  SaveAppResult,
  type ScaffoldStatus,
  StoredApp,
  canonicalJson,
  contentRefs,
  defaultHandle,
  scaffoldPhaseName,
} from "./apps.js";
import { AdvanceResult, Branch, CreateBranchResult, SnapshotResult } from "./branch.js";
import { CaptureRecord, type CaptureRecordPage } from "./capture.js";
import { Chain } from "./chains.js";
import type { DagSpecJson, DagSpecStep } from "./chains.js";
import { ContentItem, PutResult } from "./content.js";
import {
  ContextBundle,
  type ContextBundleItem,
  type ContextItemInput,
  PutContextBundleResult,
} from "./context.js";
import { RunCost } from "./cost.js";
import {
  DatasetHit,
  DatasetSummary,
  type IngestDoc,
  IngestResult,
  RetrievalMode,
} from "./datasets.js";
import {
  KxConnectError,
  KxError,
  KxFailedPrecondition,
  KxNotFound,
  KxRunFailed,
  KxUnimplemented,
  KxUsage,
  KxWaitTimeout,
  rpc,
} from "./errors.js";
import { RunScore } from "./eval.js";
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
import { INSTANCE_LEN, REF_LEN, asBytes, decode, encode } from "./hexids.js";
import { DecayReport, Memory, MemoryHit, MemoryKind, MemoryStats, StoreResult } from "./memory.js";
import { ModelLifecycleResult, ModelSummary, PullStatus } from "./models.js";
import { MoteDetail } from "./motes.js";
import { ReactTurn, type ReactTurnPage } from "./react.js";
import { RecipeForm, RecipeInfo, ScoredRecipe } from "./recipes.js";
import { ReplanRound, type ReplanRoundPage } from "./replan.js";
import { ReRankTurn, type ReRankTurnPage } from "./rerank.js";
import { Result, Run } from "./run.js";
import { RunInputs, type RunPage, RunSummary } from "./runs.js";
import { SecretNameRow, type SecretNamesPage } from "./secrets.js";
import { ServerInfo } from "./serverinfo.js";
import { type AddSkillInput, AddSkillResult, SkillForm, SkillSummary } from "./skills.js";
import { TeamMembers, type TeamSummary, teamsFromProto } from "./teams.js";
import { type MoteTelemetryPage, MoteTelemetryRow, TelemetrySummary } from "./telemetry.js";
import type { TokenChunk } from "./tokens.js";
import {
  BundleScore,
  type BundleSpec,
  type CallToolResult,
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
import {
  type RegisterTriggerInput,
  type RegisterTriggerResult,
  type SubmitTriggerResult,
  type TestTriggerResult,
  TriggerRow,
  type TriggersPage,
  triggerAuthToProto,
  triggerKindToProto,
} from "./triggers.js";
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

/** The vision recipe handle (PR-B2) — an image→text chat over a vision-capable model
 * on either engine (Ollama vision tags / llama.cpp mmproj). */
export const VISION_RECIPE_HANDLE = "kx/recipes/vision";

/** AGENTIC-VISION: the image-grounded ReAct recipe — the live agent loop PLUS a bound
 * image the served VLM reasons over on EVERY turn. Shares the `kx/recipes/react` prefix,
 * so it settles as a react CHAIN (the `isReactHandle` prefix check). */
export const REACT_VISION_RECIPE_HANDLE = "kx/recipes/react-vision";

/** RC4b AGENTIC RAG: the dataset-grounded ReAct recipe — a live agent loop whose warrant
 * grants the read-only `retrieve` tool, so the model AUTONOMOUSLY searches a corpus
 * (hybrid), reads passages, and can re-query. Invoke with `{ instruction, dataset }`;
 * shares the `kx/recipes/react` prefix, so it settles as a chain. */
export const REACT_RAG_RECIPE_HANDLE = "kx/recipes/react-rag";

/** RC4b VISION-RAG: a single grounded multimodal completion — the served VLM answers
 * about an attached image WHILE grounded on a dataset's retrieved TEXT passages (one
 * generation). `chat(prompt, { image, dataset })` binds it. Text-only datasets. */
export const VISION_RAG_RECIPE_HANDLE = "kx/recipes/vision-rag";

/**
 * An image to attach to a {@link KxClientBase.chat} call (PR-B2). Either an existing
 * `{ ref }` (a 64-hex `PutContent` ref) or raw `bytes` to upload (a bare `Uint8Array`
 * is shorthand for `{ bytes }`). Used for image→text vision AND prompted OCR
 * ("transcribe the text in this image") — both are the same vision dispatch.
 */
export type ImageInput = Uint8Array | { ref: string } | { bytes: Uint8Array; mediaType?: string };

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

/** True when this blueprint step is a MODEL step (mirrors the CLI `resolve_kind`
 *  inference: an explicit `kind`, else model fields ⇒ model). */
function isModelStep(s: DagSpecStep): boolean {
  return s.kind === "model" || (s.kind === undefined && Boolean(s.model_id || s.prompt));
}

/** Normalize a `chat({ tools })` grant list to a `{ name: version }` contract,
 * accepting BOTH the CLI `--tools` `id@version` form AND a bare `id` (→ version `"1"`,
 * the `@tool` grammar default). The map lowers to the exact tool_contract the CLI
 * builds, so `chat({ tools })`, `flow().agent({ tools })`, and `kx chat --tools` agree. */
function toolsToContract(tools: readonly string[]): Record<string, string> {
  const contract: Record<string, string> = {};
  for (const tool of tools) {
    const at = tool.lastIndexOf("@");
    const name = at > 0 ? tool.slice(0, at) : tool;
    const version = at > 0 && at < tool.length - 1 ? tool.slice(at + 1) : "1";
    if (!(name in contract)) {
      contract[name] = version;
    }
  }
  return contract;
}

/**
 * POC-5d: fold an App's `input_schema` args into the ENTRY (first) model step's
 * prompt as a clearly-delimited "Inputs" block, returning a NEW blueprint (never
 * mutates the source). A NO-OP when `args` is empty/absent OR the blueprint has no
 * model step ⇒ byte-identical to the pre-POC-5d compile. The server still
 * re-resolves every warrant from the caller's grants (SN-8); args steer, never grant.
 */
export function injectAppArgs(
  blueprint: DagSpecJson,
  args: Record<string, string> | undefined,
): DagSpecJson {
  const entries = args ? Object.entries(args).filter(([, v]) => v !== undefined) : [];
  if (entries.length === 0) return blueprint;
  const idx = (blueprint.steps ?? []).findIndex(isModelStep);
  if (idx < 0) return blueprint;
  const block = entries.map(([k, v]) => `- ${k}: ${v}`).join("\n");
  const steps = blueprint.steps.map((s, i) =>
    i === idx ? { ...s, prompt: `${s.prompt ?? ""}\n\nInputs:\n${block}`.trim() } : s,
  );
  return { ...blueprint, steps };
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

/** One step of an NL-proposed workflow (a {@link KxClientBase.proposeWorkflow} result). */
export interface ProposedWorkflowStep {
  /** The vetted role the step plays (a persona name — e.g. researcher / analyst / writer). */
  role: string;
  /** The model's per-step instruction (what this step must produce). */
  intent: string;
  /** The structural kind: `plain` | `critic` | `deterministic_critic` | `topology_shaper`. */
  kind: string;
  /** The model id the server resolved for the step (display only). */
  modelId: string;
  /** The resolved tool grant set (display only; empty for a pure model step). */
  toolContract: Record<string, string>;
}

/** One dependency edge (indices into a {@link WorkflowProposal}'s `steps`). */
export interface ProposedWorkflowEdge {
  parent: number;
  child: number;
}

/**
 * The outcome of {@link KxClientBase.proposeWorkflow}: a compiled multi-step proposal to
 * preview + confirm, or an honest rejection (no served model, an inadmissible plan, …).
 */
export type WorkflowProposal =
  | { proposed: true; steps: ProposedWorkflowStep[]; edges: ProposedWorkflowEdge[] }
  | { proposed: false; reason: string };

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
    const run = new Run(
      this,
      resp.instanceId,
      resp.terminalMoteId,
      resp.recipeFingerprint,
      resp.reactChainSalt,
    );
    if (!opts.wait) return run;
    const result =
      // React CHAIN recipes (react / react-fs / react-auto) settle via
      // ListReactTurns, not a terminal Mote (F13); they share the prefix.
      // react-edit is EXCLUDED — a single model step settling on its terminal mote.
      handle.startsWith(REACT_RECIPE_HANDLE) && handle !== "kx/recipes/react-edit"
        ? // F13: a react chain settles via ListReactTurns, not a terminal Mote.
          // PR-R1: scope the settle poll to THIS invocation's chain via reactChainSalt.
          this._finish(
            await pollReactResult(
              this.grpc,
              resp.instanceId,
              resp.terminalMoteId,
              opts.timeoutMs ?? 120_000,
              resp.reactChainSalt,
            ),
            resp.reactChainSalt.length > 0 ? encode(resp.reactChainSalt) : "",
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
   * One-shot conversational answer (POC-1) — the headline "ask a question, get a
   * string" path. A thin sugar over {@link invoke}(`{ wait: true }`): with no
   * `dataset` it binds `kx/recipes/chat` (`{ prompt }`); with `opts.dataset` it
   * binds `kx/recipes/chat-rag` (`{ prompt, dataset, k }`) so the SERVER embeds the
   * prompt, retrieves the dataset's top-`k` docs, folds them in, and answers —
   * grounding is server-side (SN-8). A missing/empty dataset HONESTLY degrades to a
   * plain answer (never faked). Returns the decoded answer string (`""` if the
   * committed payload was not UTF-8 text), reusing {@link Result.text} — the SAME
   * answer extraction as the `invoke` path. Throws {@link KxRunFailed} /
   * {@link KxWaitTimeout} on a failed / timed-out run.
   */
  async chat(
    prompt: string,
    opts: {
      dataset?: string;
      k?: number;
      timeoutMs?: number;
      image?: ImageInput;
      tools?: readonly string[];
      maxTurns?: number;
      maxToolCalls?: number;
    } = {},
  ): Promise<string> {
    // Attaching `tools` makes the turn a BOUNDED agentic (ReAct) turn — one MODEL step
    // granted ONLY those tools (the server builds the scoped warrant, SN-8; never the
    // autogrant blanket). Lowers through the same `flow().agent({ tools })` path as the CLI
    // `kx chat --tools` and scopes the wait to THIS turn's chain (reactChainSalt). Does not
    // compose with dataset/image yet (a clear usage error, never a silent drop).
    if (opts.tools && opts.tools.length > 0) {
      if (opts.dataset !== undefined || opts.image !== undefined) {
        throw new KxUsage(
          "chat({ tools }) does not compose with dataset/image yet; run them separately",
        );
      }
      const { flow } = await import("./flow.js");
      const chain = flow()
        .agent(prompt, {
          tools: toolsToContract(opts.tools),
          maxTurns: opts.maxTurns,
          maxToolCalls: opts.maxToolCalls,
        })
        .toChain();
      const result = (await this.runChain(chain, {
        wait: true,
        timeoutMs: opts.timeoutMs,
      })) as Result;
      return result.text ?? "";
    }
    // PR-B2 vision: an `image` attaches to the SAME single-entry chat call and binds
    // the `kx/recipes/vision` recipe (image→text). RC4b: `image` + `dataset` together
    // binds `kx/recipes/vision-rag` — the VLM answers about the image WHILE grounded on
    // the dataset's retrieved text (a clear usage error when vision-RAG is not
    // provisioned, never a silent drop).
    if (opts.image !== undefined) {
      const imageRef = await this.resolveImageRef(opts.image);
      const { handle, args } =
        opts.dataset !== undefined
          ? await this.bindVisionRag(prompt, imageRef, opts.dataset, opts.k ?? 4)
          : await this.bindVision(prompt, imageRef);
      const result = (await this.invoke(handle, args, {
        wait: true,
        timeoutMs: opts.timeoutMs,
      })) as Result;
      return result.text ?? "";
    }
    const dataset = opts.dataset;
    const handle = dataset ? "kx/recipes/chat-rag" : "kx/recipes/chat";
    const args: Args = dataset ? { prompt, dataset, k: opts.k ?? 4 } : { prompt };
    const result = (await this.invoke(handle, args, {
      wait: true,
      timeoutMs: opts.timeoutMs,
    })) as Result;
    return result.text ?? "";
  }

  /**
   * Resolve an {@link ImageInput} to a 64-hex `PutContent` ref (PR-B2): an existing
   * `{ ref }` passes through; raw `bytes` (or a bare `Uint8Array`) are uploaded via
   * {@link KxClientBase.putContent} and the server-derived ref returned.
   */
  private async resolveImageRef(image: ImageInput): Promise<string> {
    if (image instanceof Uint8Array) {
      return (await this.putContent(image)).contentRef;
    }
    if ("ref" in image) return image.ref;
    return (await this.putContent(image.bytes, { mediaType: image.mediaType })).contentRef;
  }

  /**
   * Bind the `kx/recipes/vision` recipe for an image-bearing chat (PR-B2): resolve the
   * published form (the SAME form-gate the console uses), pick a legal `model` ENUM
   * value, and assemble `{ prompt, image_ref, model }`. Honest-degrade: if no
   * image-capable model is served the recipe form is absent ⇒ a clear usage error
   * (never a silent text answer that ignores the image).
   */
  private async bindVision(
    prompt: string,
    imageRef: string,
  ): Promise<{ handle: string; args: Args }> {
    let form: RecipeForm;
    try {
      form = await this.getRecipeForm(VISION_RECIPE_HANDLE);
    } catch {
      throw new KxUsage(
        "vision is not available on this serve (no image-capable model). Pull/serve a vision model (e.g. gemma3 via Ollama, or Gemma-4 + mmproj via llama.cpp).",
      );
    }
    const has = (n: string) => form.fields.find((f) => f.name === n);
    if (!has("image_ref")) {
      throw new KxUsage("the kx/recipes/vision form does not declare an image_ref slot");
    }
    const args: Args = { image_ref: imageRef };
    if (has("prompt")) args.prompt = prompt;
    const model = has("model");
    if (model) {
      // The server validates ENUM membership; pre-pick a legal value so the happy
      // path never round-trips a refusal (mirrors the console's planVisionArgs).
      args.model =
        this.defaultModel !== undefined && model.allowed.includes(this.defaultModel)
          ? this.defaultModel
          : model.allowed[0];
    }
    return { handle: VISION_RECIPE_HANDLE, args };
  }

  /**
   * RC4b: bind `kx/recipes/vision-rag` — the served VLM answers about the image WHILE
   * grounded on the dataset's top-k retrieved text (`{ prompt, image_ref, model, dataset,
   * k }`; the server strips + folds `dataset`/`k`). Honest-degrade: a clear usage error
   * when vision-RAG is not provisioned (needs BOTH a vision model AND dataset/hnsw).
   */
  private async bindVisionRag(
    prompt: string,
    imageRef: string,
    dataset: string,
    k: number,
  ): Promise<{ handle: string; args: Args }> {
    let form: RecipeForm;
    try {
      form = await this.getRecipeForm(VISION_RAG_RECIPE_HANDLE);
    } catch {
      throw new KxUsage(
        "vision-RAG is not available on this serve — it needs BOTH an image-capable model AND the dataset (hnsw) features. Drop `dataset` for a plain vision answer, or serve a vision model with datasets enabled.",
      );
    }
    const has = (n: string) => form.fields.find((f) => f.name === n);
    if (!has("image_ref")) {
      throw new KxUsage("the kx/recipes/vision-rag form does not declare an image_ref slot");
    }
    const args: Args = { image_ref: imageRef, dataset, k };
    if (has("prompt")) args.prompt = prompt;
    const model = has("model");
    if (model) {
      args.model =
        this.defaultModel !== undefined && model.allowed.includes(this.defaultModel)
          ? this.defaultModel
          : model.allowed[0];
    }
    return { handle: VISION_RAG_RECIPE_HANDLE, args };
  }

  /**
   * AGENTIC-VISION: resolve `image` to a content ref and bind `kx/recipes/react-vision`
   * (the image-grounded agent loop), injecting `image_ref` into the react `args` so the
   * served VLM reasons over the image on every turn. Honest-degrades with a clear error
   * when no vision model is served — never silently drops the image (GR15). Public so the
   * standalone `runAgent` / `Agent` entrypoints can bind it.
   */
  async bindReactVision(
    args: Record<string, unknown>,
    image: ImageInput,
  ): Promise<{ handle: string; args: Record<string, unknown> }> {
    const imageRef = await this.resolveImageRef(image);
    let form: RecipeForm;
    try {
      form = await this.getRecipeForm(REACT_VISION_RECIPE_HANDLE);
    } catch {
      throw new KxUsage(
        "agentic vision is not available on this serve (no image-capable model). Serve a vision model (e.g. gemma3 via Ollama, or Gemma-4 + mmproj via llama.cpp).",
      );
    }
    if (!form.fields.find((f) => f.name === "image_ref")) {
      throw new KxUsage("the kx/recipes/react-vision form does not declare an image_ref slot");
    }
    return { handle: REACT_VISION_RECIPE_HANDLE, args: { ...args, image_ref: imageRef } };
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
  ): Promise<Run | Result> {
    fillDefaultModel(request, this.defaultModel);
    const handle = await rpc(this.grpc.submitWorkflow(request));
    // V2a: a workflow has no statically-known terminal Mote — return a {@link Run} with
    // an empty terminal whose `.wait()` resolves the FIRST committed Mote (await-any).
    // A tool-granted (agentic) workflow carries the server's `reactChainSalt` so the
    // Run/wait scopes to THIS run's ReAct chain instead of a stale/foreign committed Mote.
    if (!opts.wait) {
      return new Run(
        this,
        handle.instanceId,
        new Uint8Array(0),
        handle.recipeFingerprint,
        handle.reactChainSalt,
      );
    }
    if (handle.reactChainSalt.length > 0) {
      return this._awaitReact(handle.instanceId, handle.reactChainSalt, opts.timeoutMs ?? 120_000);
    }
    const outcome = await pollAny(this.grpc, handle.instanceId, opts.timeoutMs ?? 120_000);
    return this._finish(outcome);
  }

  /**
   * NL authoring (propose-then-confirm): turn a natural-language `goal` into a PROPOSED
   * multi-step workflow DAG. The SERVED model plans; the gateway decodes + compiles the
   * plan through the vetted planner (the model names only role + intent + edges — every
   * capability axis is server-vetted, SN-8). It VALIDATES ONLY — nothing runs until the
   * caller confirms by authoring the returned steps (e.g. via the builder → `saveApp` /
   * {@link KxClientBase.submitWorkflow}). Returns `{ proposed: false, reason }` when the
   * gateway can't plan (no served model, an inadmissible plan). An old gateway without the
   * seam throws {@link KxUnimplemented}.
   */
  async proposeWorkflow(goal: string): Promise<WorkflowProposal> {
    const resp = await rpc(this.grpc.proposeWorkflow({ goal }));
    const r = resp.result;
    if (r?.case === "plan") {
      return {
        proposed: true,
        steps: r.value.steps.map((s) => ({
          role: s.role,
          intent: s.intent,
          kind: s.kind,
          modelId: s.modelId,
          toolContract: { ...s.toolContract },
        })),
        edges: r.value.edges.map((e) => ({ parent: e.parent, child: e.child })),
      };
    }
    if (r?.case === "rejected") {
      return { proposed: false, reason: r.value.reason };
    }
    return { proposed: false, reason: "the gateway returned no proposal" };
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
  ): Promise<Run | Result> {
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

  // ----- POC-4 Apps (save / list / get / run; off-journal apps.db catalog) -----

  /**
   * Persist a `kortecx.app/v1` envelope to the caller-scoped catalog. The server
   * validates + canonicalizes it and derives `appRef` (SN-8); the envelope carries
   * NO authority. `handle` defaults to `apps/local/<sanitized-name>`. An old gateway
   * throws {@link KxUnimplemented}.
   */
  async saveApp(
    envelope: unknown,
    opts: { handle?: string; sourceDigest?: Uint8Array } = {},
  ): Promise<SaveAppResult> {
    const name = String((envelope as Record<string, unknown>)?.name ?? "app");
    const handle = opts.handle ?? defaultHandle(name);
    const envelopeJson = new TextEncoder().encode(canonicalJson(envelope));
    const resp = await rpc(
      this.grpc.saveApp({
        handle,
        envelopeJson,
        sourceDigest: opts.sourceDigest ?? new Uint8Array(),
      }),
    );
    return SaveAppResult.fromProto(resp);
  }

  /** List the caller's App catalog (deterministic handle order). */
  async listApps(): Promise<AppSummary[]> {
    const resp = await rpc(this.grpc.listApps({ limit: 0, afterHandle: "" }));
    return resp.apps.map((a) => AppSummary.fromProto(a));
  }

  /**
   * Fetch one App by handle, or `null` if not found / not owned (uniform — no
   * cross-party existence oracle).
   */
  async getApp(handle: string): Promise<StoredApp | null> {
    const resp = await rpc(this.grpc.getApp({ handle }));
    return resp.found ? StoredApp.fromProto(resp) : null;
  }

  /**
   * Fetch an App's READ-ONLY capability manifest ("what this App needs vs. what you
   * have"): its requested tools/connections/model diffed against your live policy.
   * `null` if not found / not owned (uniform — no existence oracle). The manifest gates
   * nothing; the runtime enforces the same intersection at run (SN-8). An old gateway
   * without the seam throws {@link KxUnimplemented}.
   */
  async getAppManifest(handle: string): Promise<AppManifest | null> {
    const resp = await rpc(this.grpc.getAppManifest({ handle }));
    return resp.found ? AppManifest.fromProto(resp) : null;
  }

  /**
   * Export a saved App as a portable `kortecx.appbundle/v1` archive — the canonical
   * envelope PLUS its transitive content-store closure (each blob fetched at FULL
   * size via {@link getContent}). `withData` includes RAG dataset payloads. Returns
   * the wire string; write it to a `.kxapp` file. Throws {@link KxNotFound} if the
   * App is absent, {@link KxUsage} if a referenced blob is missing.
   */
  async exportAppBundle(handle: string, opts: { withData?: boolean } = {}): Promise<string> {
    const stored = await this.getApp(handle);
    if (stored === null) throw new KxNotFound(`app ${JSON.stringify(handle)} not found`);
    const blobs = new Map<string, Uint8Array>();
    for (const ref of contentRefs(stored.envelope, opts.withData ?? false)) {
      const body = await this.getContent(ref);
      if (body.length === 0) {
        throw new KxUsage(
          `content ${ref} is missing or unreadable — cannot export a faithful bundle`,
        );
      }
      blobs.set(ref, body);
    }
    const envelope = new TextEncoder().encode(canonicalJson(stored.envelope));
    return new AppBundle(stored.appDigest, envelope, blobs).toJson();
  }

  /**
   * Import a `kortecx.appbundle/v1` archive under YOUR OWN principal (fail-closed):
   * putContent the content closure (the server re-derives + dedups each ref) then
   * saveApp with a `sourceDigest` lineage stamp. Connections/secrets never travel —
   * re-register them by name (the App fails closed at run until then). `force`
   * overwrites an existing same-handle App. Throws {@link KxUsage} on a
   * malformed/oversized/corrupt bundle or an existing App.
   */
  async importApp(bundle: string, opts: { force?: boolean } = {}): Promise<SaveAppResult> {
    const parsed = AppBundle.fromJson(bundle);
    if (parsed.blobCount() > MAX_BUNDLE_REFS) {
      throw new KxUsage(`bundle carries ${parsed.blobCount()} blobs (ceiling ${MAX_BUNDLE_REFS})`);
    }
    if (parsed.totalBlobBytes() > MAX_BUNDLE_CLOSURE_BYTES) {
      throw new KxUsage(
        `bundle closure is ${parsed.totalBlobBytes()} bytes (ceiling ${MAX_BUNDLE_CLOSURE_BYTES})`,
      );
    }
    const envelope = JSON.parse(new TextDecoder().decode(parsed.envelope)) as Record<
      string,
      unknown
    >;
    const handle = defaultHandle(String(envelope.name ?? "app"));
    if (!(opts.force ?? false) && (await this.getApp(handle)) !== null) {
      throw new KxUsage(`app ${JSON.stringify(handle)} already exists — pass force to overwrite`);
    }
    for (const [ref, body] of parsed.blobs) {
      const got = (await this.putContent(body)).contentRef;
      if (got !== ref) {
        throw new KxUsage(
          `corrupt bundle: a blob was declared as ${ref} but the store derived ${got}`,
        );
      }
    }
    return this.saveApp(envelope, { handle, sourceDigest: decode(parsed.appDigest) });
  }

  /**
   * Clone one of your Apps locally under a new name (a frozen copy; content is
   * already resident, so no transfer). Records the source's `appDigest` lineage.
   * Throws {@link KxNotFound} if the source is absent, {@link KxUsage} if the target
   * exists.
   */
  async cloneApp(handle: string, newname: string): Promise<SaveAppResult> {
    const stored = await this.getApp(handle);
    if (stored === null) throw new KxNotFound(`app ${JSON.stringify(handle)} not found`);
    const envelope = { ...stored.envelope, name: newname };
    const newHandle = defaultHandle(newname);
    if ((await this.getApp(newHandle)) !== null) {
      throw new KxUsage(
        `app ${JSON.stringify(newHandle)} already exists — choose a different newname`,
      );
    }
    const sourceDigest = stored.appDigest ? decode(stored.appDigest) : new Uint8Array();
    return this.saveApp(envelope, { handle: newHandle, sourceDigest });
  }

  // ----- Skills (add / list / show / remove; off-journal skills.db catalog) -----

  /**
   * Add (upsert) a `kortecx.skill/v1` skill to the caller-scoped catalog. The
   * server validates the manifest fail-closed (authority deny-keys), stores the
   * instructions body content-addressed, and derives `skillRef` +
   * `instructionsRef` (SN-8). A skill is a WISH bundle — adding one grants
   * nothing. An old gateway throws {@link KxUnimplemented}.
   */
  async addSkill(input: AddSkillInput): Promise<AddSkillResult> {
    const manifestJson = new TextEncoder().encode(canonicalJson(input.manifest));
    const instructionsBody = input.instructions
      ? new TextEncoder().encode(input.instructions)
      : new Uint8Array(0);
    const resp = await rpc(this.grpc.addSkill({ manifestJson, instructionsBody }));
    return AddSkillResult.fromProto(resp);
  }

  /** List the caller's skill catalog (deterministic name order). */
  async listSkills(): Promise<SkillSummary[]> {
    const resp = await rpc(this.grpc.listSkills({ limit: 0, afterName: "" }));
    return resp.skills.map((s) => SkillSummary.fromProto(s));
  }

  /**
   * Fetch one skill's form (summary + wishes with the ADVISORY `registered`
   * bit + the instructions preview), or `null` if not found / not owned
   * (uniform — no cross-party existence oracle).
   */
  async getSkillForm(name: string): Promise<SkillForm | null> {
    const resp = await rpc(this.grpc.getSkillForm({ name }));
    return SkillForm.fromProto(resp);
  }

  /** Remove a skill from the catalog. Returns `true` iff a row was removed. */
  async removeSkill(name: string): Promise<boolean> {
    const resp = await rpc(this.grpc.removeSkill({ name }));
    return resp.removed;
  }

  /**
   * Run a saved App (exactly-once). G2: prefers the server-side `RunApp` — the gateway
   * reads the validated stored envelope and honors its `references.connections` +
   * `guards.secret_scope` (so a credentialed connector, e.g. Gmail, can be dialed inside
   * the agentic loop) — and re-resolves EVERY warrant from the caller's grants (SN-8).
   * On an older server without the seam (`UNIMPLEMENTED`) it falls back to the legacy
   * client-orchestrated `GetApp` → `submitWorkflow` (which drops the references). Throws
   * {@link KxUsage} if the App is not found. `args` fold server-side into the entry model
   * step's prompt; empty/absent ⇒ byte-identical to a no-args compile. `requireApproval`
   * (opt-in, default `false`) runs the entry agentic step under the per-run HITL gate, so
   * an irreversible / world-mutating tool call pauses for an explicit grant/deny (see
   * {@link KxClient.approvals}) before it fires.
   */
  async runApp(
    handle: string,
    opts: {
      wait?: boolean;
      timeoutMs?: number;
      args?: Record<string, string>;
      requireApproval?: boolean;
    } = {},
  ): Promise<Run | Result> {
    const hasArgs = opts.args !== undefined && Object.keys(opts.args).length > 0;
    const argsBytes = hasArgs
      ? new TextEncoder().encode(JSON.stringify(opts.args))
      : new Uint8Array(0);
    try {
      const h = await rpc(
        this.grpc.runApp({
          handle,
          args: argsBytes,
          requireApproval: opts.requireApproval ?? false,
        }),
      );
      if (!opts.wait) {
        return new Run(this, h.instanceId, new Uint8Array(0), h.recipeFingerprint);
      }
      const outcome = await pollAny(this.grpc, h.instanceId, opts.timeoutMs ?? 120_000);
      return this._finish(outcome);
    } catch (e) {
      if (e instanceof KxUnimplemented) {
        if (hasArgs) {
          throw new KxUsage(
            "this server does not support runApp(args) (RunApp unavailable); upgrade the " +
              "server, or run without args",
          );
        }
        // Legacy client-orchestrated fallback. It compiles the blueprint locally and
        // DROPS references.connections + guards.secret_scope. If the App
        // actually declares integrations, refuse LOUDLY rather than silently run a
        // de-integrated workflow (the credentialed connector would never fire, and the
        // secret_scope narrowing would be lost). Only an integration-free App may take
        // the legacy path.
        const stored = await this.getApp(handle);
        if (stored === null) throw new KxUsage(`app ${handle} not found`);
        const env = stored.envelope as {
          blueprint: DagSpecJson;
          references?: { connections?: unknown[] };
          steering_config?: { guards?: { secret_scope?: unknown[] } };
        };
        const connections = env.references?.connections ?? [];
        const secretScope = env.steering_config?.guards?.secret_scope ?? [];
        if (connections.length > 0 || secretScope.length > 0) {
          throw new KxUsage(
            `app ${handle} declares integrations (references.connections / guards.secret_scope) but this server lacks runApp — refusing to run it de-integrated (the credentialed connector + secret_scope would be silently dropped). Upgrade the server (build with the mcp-gateway feature).`,
          );
        }
        const request = Chain.fromBlueprint(env.blueprint);
        return this.submitWorkflow(request, opts);
      }
      throw e;
    }
  }

  /**
   * POC-5d: the App's portable blueprint (a {@link DagSpecJson} — the agentic step
   * structure the lineage editor renders/edits). A thin convenience over
   * {@link getApp} (`envelope.blueprint`); `null` for an absent/not-owned App
   * (uniform — no existence oracle).
   */
  async getAppStructure(handle: string): Promise<DagSpecJson | null> {
    const stored = await this.getApp(handle);
    return stored === null ? null : (stored.envelope.blueprint as DagSpecJson);
  }

  /**
   * Resolve a context-item selector to `[index, item]` against `manifest`. `item`
   * is the advisory item NAME (a `string`) or a 0-based INDEX (a `number`); a name
   * with more than one match is AMBIGUOUS — pass the index. Throws {@link KxUsage}
   * (a client-side selection error) on an out-of-range index, an unknown name, or
   * an ambiguous name.
   */
  private resolveContextItem(
    manifest: ContextBundle,
    item: string | number,
  ): [number, ContextBundleItem] {
    const items = manifest.items;
    if (typeof item === "number") {
      const found = Number.isInteger(item) ? items[item] : undefined;
      if (found === undefined) {
        throw new KxUsage(
          `item index ${item} is out of range for bundle '${manifest.handle}' (${items.length} item${items.length === 1 ? "" : "s"})`,
        );
      }
      return [item, found];
    }
    const matches = items
      .map((it, i): [number, ContextBundleItem] => [i, it])
      .filter(([, it]) => it.name === item);
    if (matches.length > 1) {
      throw new KxUsage(
        `item name '${item}' is ambiguous in bundle '${manifest.handle}' (${matches.length} matches) — pass the integer index instead`,
      );
    }
    const sole = matches[0];
    if (sole === undefined) {
      throw new KxUsage(`no item named '${item}' in bundle '${manifest.handle}'`);
    }
    return sole;
  }

  /**
   * Re-read `handle` as the freshest edit base + run the optimistic-concurrency
   * guard. With `expectBundleRef` set, a mismatch means the bundle changed under
   * the caller ⇒ {@link KxFailedPrecondition} (fail-closed, never a silent
   * last-writer-wins clobber). The content-addressed `bundleRef` is a free
   * compare-and-swap token (any item/description change moves it).
   */
  private async readContextBundleOrThrow(
    handle: string,
    expectBundleRef: string | undefined,
  ): Promise<ContextBundle> {
    const manifest = await this.getContextBundle(handle);
    if (manifest === null) throw new KxError(`context bundle '${handle}' not found`);
    if (expectBundleRef !== undefined && manifest.bundleRef !== expectBundleRef) {
      throw new KxFailedPrecondition(
        `context bundle '${handle}' changed since you read it (expected bundle_ref ${expectBundleRef}, now ${manifest.bundleRef}); re-read it and re-apply your change`,
      );
    }
    return manifest;
  }

  /**
   * Replace one context-item's body IN PLACE (POC-2 context-edit). The content
   * store is IMMUTABLE, so this uploads `newBody` (a NEW server-derived ref via
   * {@link putContent}) and re-upserts the bundle with that item re-pointed at the
   * new ref — the item's advisory `name` and `mediaType` are preserved unless
   * `opts.mediaType` overrides. `item` selects by name or index. Set
   * `opts.expectBundleRef` (the `bundleRef` you viewed) to fail-closed on a
   * concurrent change ({@link KxFailedPrecondition}) instead of a silent overwrite.
   * Editing to byte-identical content re-reports `deduplicated`.
   */
  async editContextItem(
    handle: string,
    item: string | number,
    newBody: Uint8Array,
    opts: { mediaType?: string; expectBundleRef?: string } = {},
  ): Promise<PutContextBundleResult> {
    const manifest = await this.readContextBundleOrThrow(handle, opts.expectBundleRef);
    const [idx, target] = this.resolveContextItem(manifest, item);
    const media = opts.mediaType ?? target.mediaType;
    const put = await this.putContent(newBody, { mediaType: media, filename: target.name });
    const items: ContextItemInput[] = manifest.items.map((it) => ({
      name: it.name,
      contentRef: it.contentRef,
      mediaType: it.mediaType,
    }));
    items[idx] = { name: target.name, contentRef: put.contentRef, mediaType: media };
    return this.putContextBundle(handle, items, { description: manifest.description });
  }

  /**
   * Drop one item from a bundle (POC-2) and re-upsert the remainder. Refuses
   * ({@link KxUsage}) if it would empty the bundle — the server rejects an empty
   * manifest; use {@link deleteContextBundle} to unbind the whole handle.
   * `opts.expectBundleRef` makes it fail-closed on a concurrent change.
   */
  async removeContextItem(
    handle: string,
    item: string | number,
    opts: { expectBundleRef?: string } = {},
  ): Promise<PutContextBundleResult> {
    const manifest = await this.readContextBundleOrThrow(handle, opts.expectBundleRef);
    const [idx] = this.resolveContextItem(manifest, item);
    if (manifest.items.length <= 1) {
      throw new KxUsage(
        `removing the last item would empty bundle '${handle}'; use deleteContextBundle to unbind the whole handle`,
      );
    }
    const items: ContextItemInput[] = manifest.items
      .filter((_, i) => i !== idx)
      .map((it) => ({ name: it.name, contentRef: it.contentRef, mediaType: it.mediaType }));
    return this.putContextBundle(handle, items, { description: manifest.description });
  }

  /**
   * Fetch one context-item's FULL body bytes (POC-2) from the uploads scope.
   * Returns the whole payload (the single {@link getContent} read is uncapped,
   * unlike a preview-clamped batch fetch). Throws {@link KxUsage} for an
   * unknown/ambiguous item, {@link KxError} if the bundle is gone, and the RPC's
   * {@link KxPermissionDenied} if the ref is not in this party's scope.
   */
  async exportContextItem(handle: string, item: string | number): Promise<Uint8Array> {
    const manifest = await this.getContextBundle(handle);
    if (manifest === null) throw new KxError(`context bundle '${handle}' not found`);
    const [, target] = this.resolveContextItem(manifest, item);
    return this.getContent(target.contentRef);
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
   * POC-5d: the PROPOSE half of {@link editBranch} — run the `kx/recipes/react-edit`
   * loop and return the model's proposed new body together with the file's current
   * body, WITHOUT advancing the branch. The caller reviews the diff (current vs
   * proposed) and then either approves (`advanceBranch(handle, path, resultRef)`) or
   * rejects (discards — the proposed blob is a harmless content-addressed orphan).
   * The host is NEVER written. Rejects if the chain produced no committed answer or
   * an empty body (GR15 fail-closed — same guards as the one-shot {@link editBranch}).
   *
   * `opts.contextPaths` (item6): additionally attach those sibling files' bodies as
   * read-only context, so a single high-level instruction — run once per target file —
   * produces a change that stays COHERENT across a multi-artifact edit. The target file
   * is always attached first; the target's own path and unknown paths are skipped.
   * `context_refs` is already `repeated string` on the wire, so this needs no proto
   * change — the constraint was only ever the single-element array literal below.
   */
  async editBranchPropose(
    handle: string,
    path: string,
    instruction: string,
    opts: { timeoutMs?: number; contextPaths?: readonly string[] } = {},
  ): Promise<{ resultRef: string; proposedText: string; currentText: string }> {
    const branch = await this.getBranch(handle);
    if (branch === null) throw new Error(`branch '${handle}' not found`);
    const item = branch.items.find((it) => it.path === path);
    if (item === undefined) throw new Error(`path '${path}' is not in branch '${handle}'`);
    // Multi-artifact coherence (item6): resolve any sibling paths to their current refs
    // so they can ride along as context. The target file is attached FIRST (the model
    // rewrites it) and siblings AFTER, purely to keep the per-file rewrites consistent.
    const siblingRefs: string[] = [];
    for (const p of opts.contextPaths ?? []) {
      if (p === path) continue;
      const sibling = branch.items.find((it) => it.path === p);
      if (sibling !== undefined) siblingRefs.push(sibling.contentRef);
    }
    const directive =
      siblingRefs.length === 0
        ? `You are editing the file \`${path}\`. The text in the attached context below IS its exact current contents. Apply this change: ${instruction}\n\nReturn ONLY the complete, updated file contents — no commentary, no explanation, and no markdown code fences.`
        : `You are editing the file \`${path}\`. Its exact current contents are the FIRST attached context below; the remaining attachments are other project files, provided ONLY so your edit stays consistent across the App — do not return them. Apply this change: ${instruction}\n\nReturn ONLY the complete, updated contents of \`${path}\` — no commentary, no explanation, and no markdown code fences.`;
    // react-edit is a single model step; its only free param is `prompt`.
    const result = await this.invoke(
      "kx/recipes/react-edit",
      { prompt: directive },
      {
        wait: true,
        timeoutMs: opts.timeoutMs ?? 300_000,
        contextRefs: [item.contentRef, ...siblingRefs],
      },
    );
    if (!(result instanceof Result) || !result.ok || result.resultRef === null) {
      throw new Error("react-edit produced no committed answer to advance the branch to");
    }
    // Fail CLOSED on an empty edit (GR15): never propose an empty file (a
    // heavy-reasoning model can return only stripped reasoning).
    if (result.payload === null || result.payload.length === 0) {
      throw new Error(
        "react-edit produced an empty body (the model did not return file contents); the branch was NOT advanced",
      );
    }
    const current = await this.getBranchContent(handle, path);
    return {
      resultRef: result.resultRef,
      proposedText: new TextDecoder().decode(result.payload),
      currentText: current ? new TextDecoder().decode(current) : "",
    };
  }

  /**
   * D155 Phase-3: agentically edit a branch file IN-CAS. Runs the
   * `kx/recipes/react-edit` loop and advances the manifest to the new content ref
   * in one shot. The host is NEVER written. Rejects if the chain produced no
   * committed answer. (POC-5d: this is `editBranchPropose` + `advanceBranch` — the
   * react-edit directive lives in exactly one place so the committed blob bytes are
   * identical across both APIs.)
   */
  async editBranch(
    handle: string,
    path: string,
    instruction: string,
    opts: { timeoutMs?: number } = {},
  ): Promise<AdvanceResult> {
    const { resultRef } = await this.editBranchPropose(handle, path, instruction, opts);
    return this.advanceBranch(handle, path, resultRef);
  }

  /**
   * POC-5a: agentically scaffold an existing App's FIXED-skeleton project tree into
   * its CoW branch (server-side; the host is never written). Returns immediately —
   * poll {@link getScaffoldStatus} (+ {@link getBranch}) for progress. The branch
   * defaults to the App's own handle (one-App-one-branch).
   */
  async scaffoldApp(
    handle: string,
    opts: { goal?: string; branchHandle?: string } = {},
  ): Promise<{ branchHandle: string; resumed: boolean }> {
    const resp = await rpc(
      this.grpc.scaffoldApp({
        handle,
        branchHandle: opts.branchHandle ?? "",
        instruction: opts.goal ?? "",
      }),
    );
    return { branchHandle: resp.branchHandle, resumed: resp.resumed };
  }

  /** POC-5a: the live scaffold status for a branch (phase + done/pending files). */
  async getScaffoldStatus(branchHandle: string): Promise<ScaffoldStatus> {
    const resp = await rpc(this.grpc.getScaffoldStatus({ branchHandle }));
    return {
      phase: scaffoldPhaseName(resp.phase),
      filesDone: resp.filesDone,
      filesPending: resp.filesPending,
      detail: resp.detail,
    };
  }

  /**
   * POC-5a: read one App project file's body THROUGH the caller's OWN branch
   * manifest (caller-scoped). Returns `null` for an absent branch / absent path /
   * not-owned (uniform — no existence oracle).
   */
  async getBranchContent(handle: string, path: string): Promise<Uint8Array | null> {
    const resp = await rpc(this.grpc.getBranchContent({ handle, path }));
    return resp.found ? resp.payload : null;
  }

  /** POC-5b: lock the App's project branch (agentic in-CAS edits are refused). */
  async lockApp(branchHandle: string): Promise<boolean> {
    const resp = await rpc(this.grpc.lockApp({ branchHandle }));
    return resp.locked;
  }

  /** POC-5b: unlock the App's project branch (re-enable agentic edits). */
  async unlockApp(branchHandle: string): Promise<boolean> {
    const resp = await rpc(this.grpc.unlockApp({ branchHandle }));
    return resp.unlocked;
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

  /**
   * POC-3: warm a REGISTERED local model into RAM (real load). An unregistered
   * id throws {@link KxNotFound} (fail-closed — never an arbitrary path); an
   * FFI-free gateway throws {@link KxUnimplemented}. Over-capacity ⇒ honest
   * LRU-evict-oldest (sequential swap).
   */
  async loadModel(modelId: string): Promise<ModelLifecycleResult> {
    const resp = await rpc(this.grpc.loadModel({ modelId }));
    return ModelLifecycleResult.fromLoad(resp);
  }

  /**
   * POC-3: evict a REGISTERED local model from RAM (real `llama_model_free`).
   * Idempotent (`wasResident=false` if it was not loaded); an unregistered id
   * throws {@link KxNotFound}.
   */
  async offloadModel(modelId: string): Promise<ModelLifecycleResult> {
    const resp = await rpc(this.grpc.offloadModel({ modelId }));
    return ModelLifecycleResult.fromOffload(resp);
  }

  /**
   * The resolved configuration the connected gateway is running (POC-1 Settings) —
   * model, bind addresses, store paths, caps, and feature flags. Read by an
   * authenticated caller; DISPLAY-ONLY (SN-8): server-derived, never a secret
   * (`tlsEnabled` is a POSTURE flag, never the key). An old gateway without this
   * RPC throws {@link KxUnimplemented}.
   */
  async getServerInfo(): Promise<ServerInfo> {
    const resp = await rpc(this.grpc.getServerInfo({}));
    return ServerInfo.fromProto(resp);
  }

  /**
   * Model Control v2: download + RUNTIME-register a model (no restart). Pass
   * `{ ollamaTag }` to pull from the Ollama registry, OR `{ url, sha256 }` (a
   * `huggingface.co` `/resolve/` GGUF link). Returns the `modelId` to poll via
   * {@link getPullStatus}. Deny-by-default: a refusal (downloads disabled / host
   * not allowlisted / missing sha256) throws {@link KxFailedPrecondition}. HOST
   * INFRASTRUCTURE, not a client Mote (SN-8).
   */
  async pullModel(args: { ollamaTag?: string; url?: string; sha256?: string }): Promise<string> {
    if ((args.ollamaTag == null) === (args.url == null)) {
      throw new Error("pullModel requires exactly one of ollamaTag or url");
    }
    const source =
      args.ollamaTag != null
        ? { case: "ollamaTag" as const, value: args.ollamaTag }
        : { case: "url" as const, value: args.url ?? "" };
    const resp = await rpc(this.grpc.pullModel({ source, sha256: args.sha256 ?? "" }));
    if (!resp.accepted) {
      throw new KxFailedPrecondition(`pull refused: ${resp.detail}`);
    }
    return resp.modelId;
  }

  /**
   * Model Control v2: the current progress of a {@link pullModel} download +
   * registration (advisory). An unknown id throws {@link KxNotFound}.
   */
  async getPullStatus(modelId: string): Promise<PullStatus> {
    const resp = await rpc(this.grpc.getPullStatus({ modelId }));
    return PullStatus.fromProto(modelId, resp);
  }

  /**
   * Model Control v2: set the server's ACTIVE default model (an off-journal
   * advisory hint; the server never re-routes `kx/recipes/chat`). An empty
   * `modelId` CLEARS it (back to the primary). A non-served id throws
   * {@link KxNotFound}. Returns the active id after the op ("" ⇒ cleared).
   */
  async setActiveModel(modelId = ""): Promise<string> {
    const resp = await rpc(this.grpc.setActiveModel({ modelId }));
    return resp.activeModelId;
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
  async listReactTurns(
    opts: { instanceId?: string; stepSalt?: string; limit?: number } = {},
  ): Promise<ReactTurnPage> {
    const instanceId =
      opts.instanceId === undefined ? undefined : asBytes(opts.instanceId, INSTANCE_LEN);
    // PR-R1: stepSalt (hex 32B) scopes to ONE chain within a run (serve's shared
    // journal carries one chain per Invoke plus agentic-step chains).
    const stepSalt =
      opts.stepSalt === undefined || opts.stepSalt === ""
        ? undefined
        : asBytes(opts.stepSalt, REF_LEN);
    const resp = await rpc(this.grpc.listReactTurns({ instanceId, stepSalt, limit: opts.limit }));
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
   * Enumerate a live listwise LLM re-rank loop's durable turn facts (newest-first,
   * paginated; RC4c-2) — the queryable re-rank history with the enforced
   * permutation per settled turn. `instanceId` (hex) scopes to one run; absent
   * enumerates every run. The server clamps `limit` to its max page. An old
   * gateway without this RPC throws {@link KxUnimplemented}.
   */
  async listRerankTurns(
    opts: { instanceId?: string; limit?: number } = {},
  ): Promise<ReRankTurnPage> {
    const instanceId =
      opts.instanceId === undefined ? undefined : asBytes(opts.instanceId, INSTANCE_LEN);
    const resp = await rpc(this.grpc.listReRankTurns({ instanceId, limit: opts.limit }));
    return { turns: resp.turns.map((t) => ReRankTurn.fromProto(t)), hasMore: resp.hasMore };
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
   * Operator DIAGNOSTIC: fire ONE registered tool on a dialed connector live through
   * the broker (`CallMcpTool`). `args` is a JSON object string (validated against the
   * tool's inputSchema; empty ⇒ `{}`). NOT a durable agentic effect (no journal fact)
   * — the "does this connector work" check; the agentic loop fires the same tools
   * durably. SN-8 re-enforced server-side (single-grant warrant from the tool's scopes).
   */
  async callMcpTool(name: string, tool: string, args?: string): Promise<CallToolResult> {
    const resp = await rpc(
      this.grpc.callMcpTool({
        serverName: name,
        remoteName: tool,
        argsJson: args && args.trim() !== "" ? args : "{}",
      }),
    );
    return { ok: resp.ok, resultJson: resp.resultJson, error: resp.error };
  }

  /**
   * The connector (external MCP server) admin namespace — `kx.connections.add /
   * list / test / remove / discover` (the verb vocabulary of the `kx connections`
   * CLI). Each method delegates 1:1 to the flat `registerMcpServer` etc. (which
   * remain for back-compat). A connector is an external MCP tool server (see
   * `kx-extension-sdk`); chain one straight into a flow with
   * `flow().withMcp(...)`.
   */
  get connections() {
    return {
      add: (input: RegisterMcpServerInput): Promise<RegisterServerResult> =>
        this.registerMcpServer(input),
      list: (opts: { limit?: number; afterName?: string } = {}): Promise<McpServersPage> =>
        this.listMcpServers(opts),
      test: (name: string): Promise<boolean> => this.testMcpServer(name),
      remove: (name: string): Promise<boolean> => this.deregisterMcpServer(name),
      discover: (name: string): Promise<RegisteredToolsPage> => this.discoverServerTools(name),
      fire: (name: string, tool: string, args?: string): Promise<CallToolResult> =>
        this.callMcpTool(name, tool, args),
    };
  }

  /**
   * The grouped SKILLS surface — `kx.skills.add / list / show / remove`
   * (the verb vocabulary of the `kx skills` CLI). Each method delegates 1:1 to
   * the flat `addSkill` etc. A skill is a declarative `kortecx.skill/v1` bundle
   * (instructions + tool grant-WISHES) an App attaches via the builder's
   * `.skill(...)`; attaching grants nothing — the server intersects at run.
   */
  get skills() {
    return {
      add: (input: AddSkillInput): Promise<AddSkillResult> => this.addSkill(input),
      list: (): Promise<SkillSummary[]> => this.listSkills(),
      show: (name: string): Promise<SkillForm | null> => this.getSkillForm(name),
      remove: (name: string): Promise<boolean> => this.removeSkill(name),
    };
  }

  /**
   * Store a host SECRET in the local OS keychain (MM-3 / D110 `PutSecret`) under a
   * `SecretRef` NAME that a connection's / trigger's `credential_ref` points at.
   * The `value` is WRITE-ONLY — the handler stores it + drops it; it is never on a
   * read wire (D81). Gated loopback-only + an authenticated party server-side.
   * Returns `true` iff it was stored.
   */
  async putSecret(name: string, value: string): Promise<boolean> {
    const resp = await rpc(this.grpc.putSecret({ name, value }));
    return resp.stored;
  }

  /**
   * List the stored secret NAMES + audit timestamps (MM-3 `ListSecretNames`), in
   * `(name)` order. The secret VALUE is never returned (write-only).
   */
  async listSecretNames(
    opts: { limit?: number; afterName?: string } = {},
  ): Promise<SecretNamesPage> {
    const resp = await rpc(
      this.grpc.listSecretNames({ limit: opts.limit ?? 0, afterName: opts.afterName ?? "" }),
    );
    return { names: resp.names.map((s) => SecretNameRow.fromProto(s)), hasMore: resp.hasMore };
  }

  /**
   * Remove a stored secret (MM-3 `DeleteSecret`). Returns `true` iff one was removed.
   */
  async deleteSecret(name: string): Promise<boolean> {
    const resp = await rpc(this.grpc.deleteSecret({ name }));
    return resp.removed;
  }

  /**
   * The host secret store admin namespace — `kx.secrets.set / list / remove` (the
   * verb vocabulary of the `kx secrets` CLI). A `SecretRef` NAME is what a
   * connection's / trigger's `credential_ref` points at; the VALUE is write-only.
   */
  get secrets() {
    return {
      set: (name: string, value: string): Promise<boolean> => this.putSecret(name, value),
      list: (opts: { limit?: number; afterName?: string } = {}): Promise<SecretNamesPage> =>
        this.listSecretNames(opts),
      remove: (name: string): Promise<boolean> => this.deleteSecret(name),
    };
  }

  /**
   * Register a TRIGGER (D113 / D170.b `RegisterTrigger`) — bind an inbound EVENT (a
   * webhook POST, a cron interval/expression, or a bare `SubmitTrigger` RPC) to EITHER a
   * recipe handle (`recipeHandle`) OR a saved App (`appHandle` — T-APP-TRIGGER-TARGET:
   * the credentialed App fires unattended with its connections + secret_scope resolved).
   * A cron `scheduleSpec` is interval seconds (`"300"`) OR a 5-field crontab expr
   * (`"0 9 * * 1-5"`) in `timezone`. `requireApproval` adds a per-trigger HITL gate
   * (D114). The auth secret is referenced by NAME only (D81); the server derives the
   * trigger id (SN-8). Returns the trigger id (hex).
   */
  async registerTrigger(input: RegisterTriggerInput): Promise<RegisterTriggerResult> {
    const resp = await rpc(
      this.grpc.registerTrigger({
        name: input.name,
        kind: triggerKindToProto(input.kind),
        recipeHandle: input.recipeHandle ?? "",
        appHandle: input.appHandle ?? "",
        auth: triggerAuthToProto(input.auth ?? "none"),
        authSecretRef: input.authSecretRef ?? "",
        scheduleSpec: input.scheduleSpec ?? "",
        timezone: input.timezone ?? "",
        enabled: input.enabled ?? true,
        requireApproval: input.requireApproval ?? false,
      }),
    );
    return { triggerId: encode(resp.triggerId) };
  }

  /**
   * List the registered triggers (D113 `ListTriggers`), in `(name)` order. A row is
   * a governance VIEW — never a secret value (`authSecretPresent` reports only
   * whether a ref NAME is attached).
   */
  async listTriggers(opts: { limit?: number; afterName?: string } = {}): Promise<TriggersPage> {
    const resp = await rpc(
      this.grpc.listTriggers({ limit: opts.limit ?? 0, afterName: opts.afterName ?? "" }),
    );
    return { triggers: resp.triggers.map((t) => TriggerRow.fromProto(t)), hasMore: resp.hasMore };
  }

  /**
   * Remove a registered trigger (D113 `DeregisterTrigger`). Returns `true` iff one
   * was removed.
   */
  async deregisterTrigger(name: string): Promise<boolean> {
    const resp = await rpc(this.grpc.deregisterTrigger({ name }));
    return resp.removed;
  }

  /**
   * Fire a trigger (D113 `SubmitTrigger`) — the inbound EVENT verb. `payloadJson` is
   * the event body, passed as the recipe args (passthrough; empty ⇒ `{}`).
   * `idempotencyKey` dedups at the event level (empty ⇒ server-derived from the
   * payload). Returns the registered run instance id (hex) + whether a prior
   * identical event already started it.
   */
  async submitTrigger(
    name: string,
    payloadJson?: string,
    idempotencyKey?: string,
  ): Promise<SubmitTriggerResult> {
    const resp = await rpc(
      this.grpc.submitTrigger({
        name,
        idempotencyKey: idempotencyKey ?? "",
        payloadJson: payloadJson && payloadJson.trim() !== "" ? payloadJson : "{}",
      }),
    );
    return { instanceId: encode(resp.instanceId), deduped: resp.deduped };
  }

  /**
   * Dry-run a trigger (D113 `TestTrigger`) — validate the binding (the handle
   * resolves, the payload binds) WITHOUT firing. `ok` is `false` with a non-empty
   * `detail` on a binding failure.
   */
  async testTrigger(name: string, payloadJson?: string): Promise<TestTriggerResult> {
    const resp = await rpc(
      this.grpc.testTrigger({
        name,
        payloadJson: payloadJson && payloadJson.trim() !== "" ? payloadJson : "{}",
      }),
    );
    return { ok: resp.ok, detail: resp.detail };
  }

  /**
   * The trigger admin namespace — `kx.triggers.add / list / test / fire / remove`
   * (the verb vocabulary of the `kx triggers` CLI). Each method delegates 1:1 to
   * the flat `registerTrigger` etc. `kind`/`auth` are friendly string unions mapped
   * to the proto enums; ids come back hex-encoded.
   */
  get triggers() {
    return {
      add: (input: RegisterTriggerInput): Promise<RegisterTriggerResult> =>
        this.registerTrigger(input),
      list: (opts: { limit?: number; afterName?: string } = {}): Promise<TriggersPage> =>
        this.listTriggers(opts),
      test: (name: string, payload?: string): Promise<TestTriggerResult> =>
        this.testTrigger(name, payload),
      fire: (
        name: string,
        payload?: string,
        idempotencyKey?: string,
      ): Promise<SubmitTriggerResult> => this.submitTrigger(name, payload, idempotencyKey),
      remove: (name: string): Promise<boolean> => this.deregisterTrigger(name),
    };
  }

  // --- D114 (HITL approval) + M11 (cost readout) -----------------------------

  /** List the world-mutating actions withheld awaiting operator approval (D114). */
  async listPendingApprovals(limit = 0): Promise<PendingApprovalsPage> {
    const resp = await rpc(this.grpc.listPendingApprovals({ limit }));
    return { approvals: resp.approvals.map((a) => PendingApprovalRow.fromProto(a)) };
  }

  /** Grant a pending approval (D114) — releases the staged action to fire exactly
   *  once. Resolves `true` iff a decision was recorded. */
  async grantApproval(requestId: string, reason = ""): Promise<boolean> {
    const resp = await rpc(this.grpc.grantApproval({ requestId: asBytes(requestId, 16), reason }));
    return resp.granted;
  }

  /** Deny a pending approval (D114) — the gated chain dead-letters fail-closed. */
  async denyApproval(requestId: string, reason = ""): Promise<boolean> {
    const resp = await rpc(this.grpc.denyApproval({ requestId: asBytes(requestId, 16), reason }));
    return resp.denied;
  }

  /** The run's DISPLAY-ONLY local spend estimate (M11) — priced turn/tool counters. */
  async getRunCost(instanceId: string): Promise<RunCost> {
    const resp = await rpc(this.grpc.getRunCost({ instanceId: asBytes(instanceId, INSTANCE_LEN) }));
    return RunCost.fromProto(resp);
  }

  /**
   * A live run's EXPECTATION-FREE quality summary (RC1/D172, `ScoreRun`) — terminal
   * reached, turns / tool-calls spent, budget burn, rejection count. The golden-suite
   * gate (vs an expectation) runs offline via `kx eval run`.
   */
  async scoreRun(instanceId: string): Promise<RunScore> {
    const resp = await rpc(this.grpc.scoreRun({ instanceId: asBytes(instanceId, INSTANCE_LEN) }));
    return RunScore.fromProto(resp);
  }

  /**
   * The HITL approval namespace — `kx.approvals.listPending / grant / deny` (D114).
   * Grant/deny release/reject a staged world-mutating action over a server-derived
   * `requestId` (SN-8).
   */
  get approvals() {
    return {
      listPending: (limit = 0): Promise<PendingApprovalsPage> => this.listPendingApprovals(limit),
      grant: (requestId: string, reason?: string): Promise<boolean> =>
        this.grantApproval(requestId, reason),
      deny: (requestId: string, reason?: string): Promise<boolean> =>
        this.denyApproval(requestId, reason),
    };
  }

  /**
   * The cost-spend guardrail namespace — `kx.cost.getRunCost` (M11). A display-only
   * local spend estimate, not Cloud billing.
   */
  get cost() {
    return {
      getRunCost: (instanceId: string): Promise<RunCost> => this.getRunCost(instanceId),
    };
  }

  /**
   * The agentic-evaluation namespace — `client.eval.scoreRun` (RC1/D172). An
   * expectation-free per-run quality summary; the golden gate runs offline.
   */
  get eval() {
    return {
      scoreRun: (instanceId: string): Promise<RunScore> => this.scoreRun(instanceId),
    };
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
   * the `inference` feature). `mode` (RC4a) selects dense vs hybrid; `rerank` (RC4c)
   * overrides the operator's MMR diversity-rerank default per query (omitted ⇒ the
   * server default). Hits are ordered by the DISPLAY-ONLY score (SN-8). An unknown
   * dataset throws {@link KxNotFound}.
   */
  async queryDataset(
    dataset: string,
    opts: {
      text?: string;
      embedding?: readonly number[];
      k?: number;
      mode?: RetrievalMode;
      rerank?: boolean;
    } = {},
  ): Promise<DatasetHit[]> {
    const resp = await rpc(
      this.grpc.queryDataset({
        dataset,
        queryText: opts.text ?? "",
        queryEmbedding: opts.embedding ? Array.from(opts.embedding) : [],
        k: opts.k ?? 10,
        retrievalMode: opts.mode ?? RetrievalMode.UNSPECIFIED,
        rerank: opts.rerank,
      }),
    );
    return resp.hits.map((h) => DatasetHit.fromProto(h));
  }

  // -- RC5a: durable agentic memory (also via the `memory` accessor) --

  /**
   * Remember a fact for LATER runs to recall (RC5a). Content-addressed + idempotent
   * (the same fact dedups to one memory). The SDK uses the SERVER-EMBED path, so the
   * gateway needs `inference,hnsw` + a model + `KX_SERVE_MEMORY=1` (else
   * {@link KxUnimplemented} / {@link KxFailedPrecondition}). Scoped to the caller's
   * own principal.
   */
  async storeMemory(
    content: string | Uint8Array,
    opts: { kind?: MemoryKind } = {},
  ): Promise<StoreResult> {
    const body = typeof content === "string" ? new TextEncoder().encode(content) : content;
    const resp = await rpc(
      this.grpc.storeMemory({
        content: body,
        kind: opts.kind ?? MemoryKind.UNSPECIFIED,
        namespace: "",
      }),
    );
    return StoreResult.fromProto(resp);
  }

  /**
   * The episodic memory log, newest-first, optionally scoped to one run
   * (`instanceId` hex). An old / memory-less gateway throws {@link KxUnimplemented}.
   */
  async listMemories(
    opts: { instanceId?: string; limit?: number; includeTombstoned?: boolean } = {},
  ): Promise<Memory[]> {
    const resp = await rpc(
      this.grpc.listMemories({
        limit: opts.limit,
        instanceId: opts.instanceId ? decode(opts.instanceId) : undefined,
        namespace: "",
        includeTombstoned: opts.includeTombstoned ?? false,
      }),
    );
    return resp.memories.map((m) => Memory.fromProto(m));
  }

  /**
   * Recall the top-`k` memories most similar to `text` (RC5a). Each hit's `score` is
   * DISPLAY-ONLY (SN-8). Scoped to the caller's own principal.
   */
  async recallMemory(text: string, opts: { k?: number } = {}): Promise<MemoryHit[]> {
    const resp = await rpc(
      this.grpc.recallMemory({ queryText: text, k: opts.k ?? 5, namespace: "" }),
    );
    return resp.hits.map((h) => MemoryHit.fromProto(h));
  }

  /**
   * Erase a memory by its content id (hex). Returns `true` if a row was removed.
   * Scoped to the caller's own principal.
   */
  async forgetMemory(memoryId: string): Promise<boolean> {
    const resp = await rpc(this.grpc.forgetMemory({ memoryId: decode(memoryId), namespace: "" }));
    return resp.forgotten;
  }

  /**
   * Preview (`dryRun`, the default) or apply a reversible TTL+salience decay sweep
   * (RC5b). A candidate is older than `ttlDays` AND recalled fewer than `minAccess`
   * times; evictions are soft-tombstones (the row is never deleted — restore via
   * {@link restoreMemory}). Scoped to the caller's own principal.
   */
  async decayMemory(
    opts: { ttlDays?: number; minAccess?: number; dryRun?: boolean } = {},
  ): Promise<DecayReport> {
    const resp = await rpc(
      this.grpc.decayMemory({
        namespace: "",
        ttlDays: opts.ttlDays ?? 90,
        minAccess: opts.minAccess ?? 1,
        dryRun: opts.dryRun ?? true,
      }),
    );
    return DecayReport.fromProto(resp);
  }

  /** Namespace memory statistics (RC5b) — live counts by kind, tombstoned count, dim,
   *  embed fingerprint, and the live age range. */
  async memoryStats(): Promise<MemoryStats> {
    const resp = await rpc(this.grpc.memoryStats({ namespace: "" }));
    return MemoryStats.fromProto(resp);
  }

  /** Un-decay (restore) a soft-tombstoned memory by its content id (hex, RC5b).
   *  Returns `true` if a tombstone was cleared. */
  async restoreMemory(memoryId: string): Promise<boolean> {
    const resp = await rpc(this.grpc.restoreMemory({ memoryId: decode(memoryId), namespace: "" }));
    return resp.restored;
  }

  /**
   * Consolidate recent episodic memories into ONE durable semantic fact (RC5b).
   * `dryRun` (the default) is a model-free PREVIEW returning the episodic memories that
   * WOULD be consolidated. `dryRun: false` drives a react-memory chain (needs a served
   * model + `KX_SERVE_MEMORY=1`) that bundles → distills → remembers, returning the
   * committed {@link Result}.
   */
  async consolidateMemory(
    opts: {
      query?: string;
      k?: number;
      windowHours?: number;
      dryRun?: boolean;
      timeoutMs?: number;
    } = {},
  ): Promise<Memory[] | Result> {
    const k = opts.k ?? 16;
    if (opts.dryRun ?? true) {
      const preview = await this.listMemories({ limit: Math.max(1, k * 4) });
      const cutoff = opts.windowHours ? Date.now() - opts.windowHours * 3_600_000 : undefined;
      return preview
        .filter((m) => m.kind === "episodic" && (cutoff === undefined || m.createdMs >= cutoff))
        .slice(0, k);
    }
    const focus = opts.query ? ` about "${opts.query}"` : "";
    const window = opts.windowHours ? ` from the last ${opts.windowHours} hours` : "";
    // Phrased to FORCE tool use: an OSS model otherwise answers from guesswork at turn 0
    // (it cannot see its episodic memories until it calls `consolidate`).
    const instruction = `You have episodic memories from earlier that you CANNOT see until you retrieve them. FIRST call the \`consolidate\` tool to bundle your recent episodic memories${focus}${window}. THEN distill the key durable facts and call \`remember\` with kind="semantic" to save ONE concise summary. Only AFTER remembering, report what you consolidated. Do NOT answer from guesswork — you must use the tools.`;
    return (await this.invoke(
      "kx/recipes/react-memory",
      { instruction, max_turns: 6, max_tool_calls: 4 },
      { wait: true, timeoutMs: opts.timeoutMs },
    )) as Result;
  }

  /**
   * The durable agentic MEMORY namespace (RC5a/RC5b) — `memory.store / list / recall /
   * forget / decay / stats / restore / consolidate` (the verb vocabulary of the `kx
   * memory` CLI). Cross-run, per-principal memory the agent recalls in later runs.
   * Chain seed facts into a flow with `flow().withMemory(...)`.
   */
  get memory() {
    return {
      store: (
        content: string | Uint8Array,
        opts: { kind?: MemoryKind } = {},
      ): Promise<StoreResult> => this.storeMemory(content, opts),
      list: (
        opts: { instanceId?: string; limit?: number; includeTombstoned?: boolean } = {},
      ): Promise<Memory[]> => this.listMemories(opts),
      recall: (text: string, opts: { k?: number } = {}): Promise<MemoryHit[]> =>
        this.recallMemory(text, opts),
      forget: (memoryId: string): Promise<boolean> => this.forgetMemory(memoryId),
      decay: (
        opts: { ttlDays?: number; minAccess?: number; dryRun?: boolean } = {},
      ): Promise<DecayReport> => this.decayMemory(opts),
      stats: (): Promise<MemoryStats> => this.memoryStats(),
      restore: (memoryId: string): Promise<boolean> => this.restoreMemory(memoryId),
      consolidate: (
        opts: {
          query?: string;
          k?: number;
          windowHours?: number;
          dryRun?: boolean;
          timeoutMs?: number;
        } = {},
      ): Promise<Memory[] | Result> => this.consolidateMemory(opts),
    };
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
    opts: { text?: string; embedding?: readonly number[]; k?: number; mode?: RetrievalMode } = {},
  ): Promise<FuzzyHit[]> {
    const resp = await rpc(
      this.grpc.fuzzyDiscovery({
        dataset,
        queryText: opts.text ?? "",
        queryEmbedding: opts.embedding ? Array.from(opts.embedding) : [],
        k: opts.k ?? 10,
        retrievalMode: opts.mode ?? RetrievalMode.UNSPECIFIED,
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

  /** Wait for the FIRST committed Mote — the `submitWorkflow` / `runChain` path, which
   * has no statically-known terminal (backs {@link Run.wait} for a workflow run). */
  async _awaitAny(instance: Uint8Array, timeoutMs: number): Promise<Result> {
    return this._finish(await pollAny(this.grpc, instance, timeoutMs));
  }

  /** Wait for an AGENTIC run's ReAct chain to settle, scoped by `salt` (a tool-granted
   * MODEL step has no static terminal). Scopes the `listReactTurns` settle poll to THIS
   * run's chain so a repeated agentic turn never reads a stale/foreign answer. Backs
   * {@link Run.wait} + `submitWorkflow(wait)` for a tool-granted run. */
  async _awaitReact(instance: Uint8Array, salt: Uint8Array, timeoutMs: number): Promise<Result> {
    const outcome = await pollReactResult(this.grpc, instance, new Uint8Array(0), timeoutMs, salt);
    return this._finish(outcome, salt.length > 0 ? encode(salt) : "");
  }

  protected _finish(outcome: WaitOutcome, reactChainSalt = ""): Result {
    const result = Result.fromOutcome(outcome, reactChainSalt);
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
