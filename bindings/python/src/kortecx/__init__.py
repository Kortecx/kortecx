"""kortecx — Python client SDK for the durable agentic-execution runtime.

A pure gRPC client over the frozen ``KxGateway`` contract. Import the clients and
go::

    from kortecx import KxClient

    with KxClient("http://127.0.0.1:50151") as kx:
        result = kx.invoke("kx/recipes/echo", {"topic": "hello"}, wait=True)
        print(result.text)
"""

from __future__ import annotations

from .alerts import AlertsPage, AlertSummary
from .blueprints import BlueprintBuilder, EdgeInput, StepInput
from .capture import CaptureRecord, CaptureRecordPage
from .chains import Chain, Task, chain, model, pure, tool
from .client import DEFAULT_ENDPOINT, AsyncKxClient, KxClient
from .content import ContentItem, PutResult
from .context import ContextBundle, ContextBundleItem, PutContextBundleResult
from .datasets import DatasetHit, DatasetSummary, FuzzyHit, IngestDocument, IngestResult
from .errors import (
    ErrorCode,
    KxCatchupRequired,
    KxConnectError,
    KxError,
    KxFailedPrecondition,
    KxInternal,
    KxInvalidArgument,
    KxNotFound,
    KxPermissionDenied,
    KxRunFailed,
    KxUnauthenticated,
    KxUnavailable,
    KxUnimplemented,
    KxUsage,
    KxWaitTimeout,
)
from .feedback import FeedbackPage, FeedbackRow, rating_from_proto, rating_to_proto
from .grants import AssetGrants, GrantView
from .models import ModelSummary
from .motes import MoteConfigItem, MoteDetail, effect_pattern_name, nd_class_name
from .react import ReactTurn, ReactTurnPage
from .recipes import (
    BlueprintForm,
    BlueprintFormField,
    RecipeForm,
    RecipeFormField,
    RecipeInfo,
    ScoredRecipe,
    blueprint_param_type_name,
    recipe_param_type_name,
)
from .replan import ReplanRound, ReplanRoundPage
from .run import AsyncRun, Result, Run
from .runs import RunInputs, RunPage, RunSummary
from .teams import TeamMember, TeamMembers, TeamSummary, WarrantView
from .telemetry import ModelTokenRollup, MoteTelemetryRow, TelemetryPage, TelemetrySummary
from .toolscout import (
    BundleScore,
    BundleSpec,
    BundleTool,
    KeywordSet,
    ManifestScore,
    McpServer,
    McpServersPage,
    RegisteredTool,
    RegisteredToolsPage,
    RegisterServerResult,
    ToolManifest,
    ToolParam,
    lower_verdict_name,
)
from .types import (
    Delta,
    Frame,
    GlobalDelta,
    MoteView,
    ParentEdge,
    Projection,
    SignatureSummary,
    TokenChunk,
    edge_kind_name,
    state_name,
)
from .wait import WaitOutcome, WaitState

__version__ = "0.1.0"

__all__ = [
    "__version__",
    "DEFAULT_ENDPOINT",
    # clients
    "KxClient",
    "AsyncKxClient",
    # run + result
    "Run",
    "AsyncRun",
    "Result",
    "WaitOutcome",
    "WaitState",
    # views
    "Projection",
    "MoteView",
    "ParentEdge",
    "Delta",
    "Frame",
    "TokenChunk",
    "SignatureSummary",
    "RunSummary",
    "RunPage",
    "RunInputs",
    "ReactTurn",
    "ReactTurnPage",
    "ReplanRound",
    "ReplanRoundPage",
    "CaptureRecord",
    "CaptureRecordPage",
    "RecipeForm",
    "RecipeInfo",
    "ScoredRecipe",
    "RecipeFormField",
    "recipe_param_type_name",
    # Blueprint = the display name for the frozen `recipe` wire (D136; aliases)
    "BlueprintForm",
    "BlueprintFormField",
    "blueprint_param_type_name",
    # The Blueprint BUILDER (SubmitWorkflow) — author a Tier-1 DAG to run.
    "BlueprintBuilder",
    "StepInput",
    "EdgeInput",
    # The Chains DSL — compose task handles into a DAG via operators or a string
    # expression, then `run_chain` it (lowers to SubmitWorkflow via the builder).
    "Chain",
    "Task",
    "chain",
    "pure",
    "model",
    "tool",
    "TeamSummary",
    "TeamMember",
    "TeamMembers",
    "WarrantView",
    "GrantView",
    "AssetGrants",
    "DatasetSummary",
    "ContentItem",
    "PutResult",
    "ContextBundle",
    "ContextBundleItem",
    "PutContextBundleResult",
    "ModelSummary",
    # Batch B: per-mote definition inspection (display-only)
    "MoteDetail",
    "MoteConfigItem",
    "nd_class_name",
    "effect_pattern_name",
    # Batch C: the cross-run global event tail + mote execution telemetry
    # (monitoring; audit/display only — never truth, never identity)
    "GlobalDelta",
    "MoteTelemetryRow",
    "TelemetryPage",
    # W1a-3: the exact per-model token-economy rollup (audit/display; token-only)
    "ModelTokenRollup",
    "TelemetrySummary",
    # W1a-2: the operator alerts inbox (read-only view; terminal-failure facts)
    "AlertSummary",
    "AlertsPage",
    # PR-4.1: user 👍/👎 feedback (advisory product signal; rebuildable-to-empty)
    "FeedbackRow",
    "FeedbackPage",
    "rating_to_proto",
    "rating_from_proto",
    "DatasetHit",
    "FuzzyHit",
    "IngestResult",
    "IngestDocument",
    # W1.A5 toolscout (advisory/display-only — scores never authorize, SN-8)
    "ToolManifest",
    "KeywordSet",
    "ManifestScore",
    "BundleScore",
    "BundleSpec",
    "BundleTool",
    "lower_verdict_name",
    # PR-6b-1 external MCP gateway (RegisterMcpServer / ListMcpServers)
    "McpServer",
    "McpServersPage",
    "RegisterServerResult",
    # PR-6a declarative tools registry (DiscoverTools / RegisterTool)
    "RegisteredTool",
    "RegisteredToolsPage",
    "ToolParam",
    "state_name",
    "edge_kind_name",
    # errors
    "ErrorCode",
    "KxError",
    "KxUnauthenticated",
    "KxPermissionDenied",
    "KxNotFound",
    "KxInvalidArgument",
    "KxUnimplemented",
    "KxUnavailable",
    "KxFailedPrecondition",
    "KxCatchupRequired",
    "KxInternal",
    "KxConnectError",
    "KxWaitTimeout",
    "KxRunFailed",
    "KxUsage",
]
