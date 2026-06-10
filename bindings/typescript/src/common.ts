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
  SignatureSummary,
  stateName,
  isCommitted,
  isPending,
} from "./types.js";

export { ParentEdge, edgeKindName } from "./parents.js";
export type { EdgeKindName } from "./parents.js";

export { RunSummary } from "./runs.js";
export type { RunPage } from "./runs.js";

export { ReactTurn } from "./react.js";
export type { ReactTurnPage } from "./react.js";

export { ReplanRound } from "./replan.js";
export type { ReplanRoundPage } from "./replan.js";

export { RecipeForm, RecipeFormField, recipeParamTypeName } from "./recipes.js";
export type { RecipeParamTypeName } from "./recipes.js";

export { TeamSummary, TeamMember, TeamMembers, WarrantView, teamsFromProto } from "./teams.js";
export { GrantView, AssetGrants } from "./grants.js";

export { DatasetSummary, DatasetHit, IngestResult } from "./datasets.js";
export type { IngestDoc } from "./datasets.js";

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
