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

export { TokenChunk } from "./tokens.js";

export { ReplanRound } from "./replan.js";
export type { ReplanRoundPage } from "./replan.js";

export { CaptureRecord } from "./capture.js";
export type { CaptureRecordPage } from "./capture.js";

// Batch C: mote execution telemetry (audit/display-only exhaust).
export { MoteTelemetryRow } from "./telemetry.js";
export type { MoteTelemetryPage } from "./telemetry.js";

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
  ChainParseError,
  ChainUnknownHandleError,
  ChainCycleError,
} from "./chains.js";
export type { Frag, ChainOptions, Lowered, LoweredStep, LoweredEdge } from "./chains.js";

export { TeamSummary, TeamMember, TeamMembers, WarrantView, teamsFromProto } from "./teams.js";
export { GrantView, AssetGrants } from "./grants.js";

export { DatasetSummary, DatasetHit, IngestResult } from "./datasets.js";
export type { IngestDoc } from "./datasets.js";

// Batch A: client uploads + batch content reads + model discovery.
export { PutResult, ContentItem } from "./content.js";
export { ModelSummary } from "./models.js";

// Batch B: per-mote definition inspection (display-only).
export { MoteDetail, MoteConfigItem, ndClassName, effectPatternName } from "./motes.js";

export {
  ToolManifest,
  KeywordSet,
  ManifestScore,
  BundleScore,
  lowerVerdictName,
  bundleSpecToProto,
} from "./toolscout.js";
export type {
  LowerVerdictName,
  BundleSpec,
  BundleToolInput,
  KeywordSetInput,
} from "./toolscout.js";

export { WaitState } from "./wait.js";
export type { WaitOutcome, WaitMode } from "./wait.js";

export { KxClientBase } from "./client.js";
export type { KxClientOptions, InvokeOptions, Id } from "./client.js";

export { DEFAULT_ENDPOINT, isNonloopbackPlaintext } from "./transport.js";
export type { Args } from "./transport.js";

export * as hexids from "./hexids.js";
export { encode, decode, asBytes, INSTANCE_LEN, REF_LEN } from "./hexids.js";

/** The generated protobuf message types + schemas (for advanced `submitRun` use). */
export * as proto from "./gen/kortecx/v1/gateway_pb.js";

export const VERSION = "0.1.0";
