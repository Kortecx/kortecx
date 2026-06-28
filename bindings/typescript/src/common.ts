/**
 * Shared, platform-neutral exports (everything except the concrete `KxClient`,
 * which is supplied by the `./node` and `./web` entrypoints). Re-exported by both.
 */

export {
  ErrorCode,
  KxError,
  KxUnauthenticated,
  KxPermissionDenied,
  KxNotFound,
  KxInvalidArgument,
  KxUnimplemented,
  KxUnavailable,
  KxFailedPrecondition,
  KxCatchupRequired,
  KxInternal,
  KxConnectError,
  KxWaitTimeout,
  KxRunFailed,
  KxUsage,
  fromRpcError,
} from "./errors.js";

export { Run, Result } from "./run.js";
export type { ResultState } from "./run.js";

export {
  Projection,
  MoteView,
  Delta,
  Frame,
  GlobalDelta,
  SignatureSummary,
  stateName,
  isCommitted,
  isPending,
} from "./types.js";

export { ParentEdge, edgeKindName } from "./parents.js";
export type { EdgeKindName } from "./parents.js";

export { RunInputs, RunSummary } from "./runs.js";
export type { RunPage } from "./runs.js";

export { ReactTurn } from "./react.js";
export type { ReactTurnPage } from "./react.js";

// PR-9c-1: the embeddable agent-runner's result (answer + audited actions). Pure
// data (no Node imports) — safe in both the node and web bundles. `runAgent` itself
// (the zero-config entry) is node-only and exported from the root `index`.
export { AgentResult, AuditedAction, assembleActions } from "./agent-result.js";
// T-AGENT2: decode a committed CriticVerdict (the kx/recipes/judge terminal).
export { decodeCriticVerdict } from "./critic.js";

export { TokenChunk } from "./tokens.js";

export { ReplanRound } from "./replan.js";
export type { ReplanRoundPage } from "./replan.js";

export { CaptureRecord } from "./capture.js";
export type { CaptureRecordPage } from "./capture.js";

// Batch C: mote execution telemetry (audit/display-only exhaust).
export { MoteTelemetryRow } from "./telemetry.js";
export type { MoteTelemetryPage } from "./telemetry.js";

// W1a-3: the exact per-model token-economy rollup (token-only; no cost/$).
export { ModelTokenRollup, TelemetrySummary } from "./telemetry.js";

// W1a-2: the operator alerts inbox (read-only view; terminal-failure facts).
export { AlertSummary } from "./alerts.js";
export type { AlertsPage } from "./alerts.js";

// PR-4.1: user 👍/👎 feedback (advisory product signal; rebuildable-to-empty).
export { FeedbackRow, ratingFromProto, ratingToProto } from "./feedback.js";
export type { FeedbackInput, FeedbackPage, Rating } from "./feedback.js";

export {
  RecipeForm,
  RecipeFormField,
  RecipeInfo,
  ScoredRecipe,
  recipeParamTypeName,
} from "./recipes.js";
export type { RecipeParamTypeName } from "./recipes.js";
// Blueprint = the display name for the frozen `recipe` wire (D136; additive aliases).
export { BlueprintForm, BlueprintFormField, blueprintParamTypeName } from "./recipes.js";
export type { BlueprintParamTypeName } from "./recipes.js";
// The Blueprint BUILDER (SubmitWorkflow) — author a Tier-1 DAG to run.
export { BlueprintBuilder } from "./blueprints.js";
export type { StepKind, ExecutionMode, EdgeType, StepInput, EdgeInput } from "./blueprints.js";

// The Chains DSL — compose task handles into a DAG (string DSL + combinators),
// lowering to the BlueprintBuilder (the cross-surface contract; see SPEC.md).
export {
  Task,
  Chain,
  ChainFrag,
  task,
  seq,
  par,
  group,
  chain,
  chainFrom,
  TOOL_ARGS_KEY,
  REACT_MAX_TURNS_KEY,
  REACT_MAX_TOOL_CALLS_KEY,
  ChainParseError,
  ChainUnknownHandleError,
  ChainCycleError,
} from "./chains.js";
export type {
  Frag,
  ChainOptions,
  Lowered,
  LoweredStep,
  LoweredEdge,
  ToolRef,
  // Batch B (D161.2): the portable blueprint export/import shapes.
  DagSpecJson,
  DagSpecStep,
} from "./chains.js";
// Batch V2 — the fluent builder + first-class Agent (the headline authoring surface).
export { Flow, flow } from "./flow.js";
export type { FlowItem, AgentStepOptions, FlowClient } from "./flow.js";
// POC-4 — the App builder + envelope (kortecx.app/v1) + catalog views.
export { app, AppBuilder, minimalAppEnvelope } from "./app.js";
export type { BlueprintSource, AppClient } from "./app.js";
export {
  APP_SCHEMA,
  AppSummary,
  SaveAppResult,
  StoredApp,
  canonicalJson,
  prettyJson,
  defaultHandle,
  scaffoldPhaseName,
} from "./apps.js";
export type { Skill, ScaffoldPhase, ScaffoldStatus } from "./apps.js";
export { Agent, REACT_RECIPE_HANDLE, REACT_AUTO_RECIPE_HANDLE } from "./agent.js";
export type { AgentOptions, AgentClient } from "./agent.js";
// V2b — local function tools (localTool → a governed stdio MCP tool the runtime dials).
export { localTool, isLocalTool, KxToolError } from "./tools.js";
export type { LocalToolDef, LocalToolSpec, LocalParamType, LocalParamSpec } from "./tools.js";

export { TeamSummary, TeamMember, TeamMembers, WarrantView, teamsFromProto } from "./teams.js";
export { GrantView, AssetGrants } from "./grants.js";

export { DatasetSummary, DatasetHit, IngestResult } from "./datasets.js";
export type { IngestDoc } from "./datasets.js";
export { FuzzyHit } from "./fuzzy.js";

// Batch A: client uploads + batch content reads + model discovery.
export { PutResult, ContentItem } from "./content.js";
// PR-7: context bundles.
export { ContextBundle, ContextBundleItem, PutContextBundleResult } from "./context.js";
export type { ContextItemInput } from "./context.js";
// D155: branches (content-addressed file branches).
export {
  AdvanceResult,
  Branch,
  BranchItem,
  CreateBranchResult,
  SnapshotResult,
} from "./branch.js";
export { ModelLifecycleResult, ModelSummary, PullStatus } from "./models.js";
// POC-1: the resolved gateway configuration view (Settings; display-only, SN-8).
export { ServerInfo } from "./serverinfo.js";

// Batch B: per-mote definition inspection (display-only).
export { MoteDetail, MoteConfigItem, ndClassName, effectPatternName } from "./motes.js";

export {
  ToolManifest,
  KeywordSet,
  ManifestScore,
  BundleScore,
  McpServer,
  RegisteredTool,
  lowerVerdictName,
  bundleSpecToProto,
} from "./toolscout.js";
export type {
  LowerVerdictName,
  BundleSpec,
  BundleToolInput,
  CallToolResult,
  KeywordSetInput,
  McpServersPage,
  RegisterMcpServerInput,
  RegisterServerResult,
  RegisteredToolsPage,
  RegisterToolInput,
  ToolParam,
} from "./toolscout.js";

// MM-3 / D110: the local OS-keychain secret store (write-only value; names-only read).
export { SecretNameRow } from "./secrets.js";
export type { SecretNamesPage } from "./secrets.js";

// D114 HITL approval gate + M11 cost-spend (operator control plane).
export { PendingApprovalRow } from "./approvals.js";
export type { PendingApprovalsPage } from "./approvals.js";
export { RunCost } from "./cost.js";

// D113 / D170.b: the local trigger admin (webhook/cron/grpc → recipe handle).
export {
  TriggerRow,
  triggerKindToProto,
  triggerKindName,
  triggerAuthToProto,
  triggerAuthName,
} from "./triggers.js";
export type {
  TriggerKindInput,
  TriggerKindName,
  TriggerAuthInput,
  TriggerAuthName,
  RegisterTriggerInput,
  RegisterTriggerResult,
  TriggersPage,
  SubmitTriggerResult,
  TestTriggerResult,
} from "./triggers.js";

export { WaitState } from "./wait.js";
export type { WaitOutcome, WaitMode } from "./wait.js";

export { KxClientBase, VISION_RECIPE_HANDLE } from "./client.js";
export type { KxClientOptions, InvokeOptions, Id, ImageInput } from "./client.js";

export { DEFAULT_ENDPOINT, isNonloopbackPlaintext } from "./transport.js";
export type { Args } from "./transport.js";

export * as hexids from "./hexids.js";
export { encode, decode, asBytes, INSTANCE_LEN, REF_LEN } from "./hexids.js";

/** The generated protobuf message types + schemas (for advanced `submitRun` use). */
export * as proto from "./gen/kortecx/v1/gateway_pb.js";

export const VERSION = "0.1.0";
