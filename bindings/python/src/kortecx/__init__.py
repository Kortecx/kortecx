"""kortecx — Python client SDK for the durable agentic-execution runtime.

A pure gRPC client over the frozen ``KxGateway`` contract. Import the clients and
go::

    from kortecx import KxClient

    with KxClient("http://127.0.0.1:50151") as kx:
        result = kx.invoke("kx/recipes/echo", {"topic": "hello"}, wait=True)
        print(result.text)
"""

from __future__ import annotations

from .agent import Agent
from .agent_result import AgentResult, AuditedAction
from .alerts import AlertsPage, AlertSummary
from .app import App, app, minimal_app_envelope
from .approvals import PendingApproval, PendingApprovalsPage
from .apps import AppSummary, SaveAppResult, ScaffoldLaunch, ScaffoldStatus, Skill, StoredApp
from .blueprints import BlueprintBuilder, EdgeInput, StepInput
from .branch import (
    AdvanceResult,
    Branch,
    BranchItem,
    CreateBranchResult,
    EditProposal,
    SnapshotResult,
)
from .capture import CaptureRecord, CaptureRecordPage
from .chains import Chain, Task, chain, model, pure
from .client import (
    DEFAULT_ENDPOINT,
    REACT_RAG_RECIPE_HANDLE,
    VISION_RAG_RECIPE_HANDLE,
    VISION_RECIPE_HANDLE,
    AsyncKxClient,
    ImageInput,
    KxClient,
)
from .content import ContentItem, PutResult
from .context import ContextBundle, ContextBundleItem, PutContextBundleResult
from .cost import RunCost
from .critic import decode_critic_verdict
from .datasets import (
    DatasetHit,
    DatasetSummary,
    FuzzyHit,
    IngestDocument,
    IngestResult,
    RetrievalMode,
)
from .defaults import (
    chat,
    default_client,
    invoke,
    make_client,
    run,
    set_default_client,
)
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
from .eval import RunScore
from .feedback import FeedbackPage, FeedbackRow, rating_from_proto, rating_to_proto
from .flow import (
    Flow,
    fan_out_gather,
    flow,
    map_reduce,
    supervisor,
    swarm,
    team,
)
from .grants import AssetGrants, GrantView
from .memory import (
    DecayCandidate,
    DecayReport,
    Memory,
    MemoryHit,
    MemoryKind,
    MemoryStats,
    StoreResult,
)
from .models import ModelLifecycleResult, ModelSummary, PullStatus
from .motes import MoteConfigItem, MoteDetail, effect_pattern_name, nd_class_name
from .personas import PERSONAS, persona, persona_names
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
from .rerank import ReRankTurn, ReRankTurnPage
from .run import AsyncRun, Result, Run
from .run_agent import run_agent, run_agent_async
from .runs import RunInputs, RunPage, RunSummary
from .secrets import SecretName, SecretNamesPage
from .server_info import ServerInfo
from .skills import AddSkillResult, SkillForm, SkillSummary, SkillWish
from .teams import TeamMember, TeamMembers, TeamSummary, WarrantView
from .telemetry import ModelTokenRollup, MoteTelemetryRow, TelemetryPage, TelemetrySummary
from .tools import LocalToolDef, ToolError, tool
from .toolscout import (
    BundleScore,
    BundleSpec,
    BundleTool,
    CallToolResult,
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
from .triggers import (
    TriggersPage,
    TriggerView,
    trigger_auth_name,
    trigger_auth_to_proto,
    trigger_kind_name,
    trigger_kind_to_proto,
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
    # PR-B2 vision
    "VISION_RECIPE_HANDLE",
    # RC4b agentic RAG
    "REACT_RAG_RECIPE_HANDLE",
    "VISION_RAG_RECIPE_HANDLE",
    "ImageInput",
    # POC-4 Apps (kortecx.app/v1 envelopes)
    "app",
    "App",
    "Skill",
    "SkillForm",
    "SkillSummary",
    "SkillWish",
    "AddSkillResult",
    "AppSummary",
    "SaveAppResult",
    "StoredApp",
    # POC-5a App scaffold + IDE
    "minimal_app_envelope",
    "ScaffoldLaunch",
    "ScaffoldStatus",
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
    "ReRankTurn",
    "ReRankTurnPage",
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
    # Batch V2 — the fluent builder + first-class Agent + zero-config helpers.
    "Flow",
    "flow",
    "Agent",
    # Multi-agent swarm authoring + a curated persona library (pure client
    # composition: N parallel agentic leaves → gather; personas fold into PROMPT_KEY).
    "swarm",
    "team",
    "fan_out_gather",
    "map_reduce",
    "supervisor",
    "persona",
    "persona_names",
    "PERSONAS",
    # PR-9c-1 — the embeddable agent-runner (goal → answer + audited actions).
    "run_agent",
    "run_agent_async",
    "AgentResult",
    "AuditedAction",
    # T-AGENT2 — decode a committed CriticVerdict (the kx/recipes/judge terminal).
    "decode_critic_verdict",
    # V2b — local function tools (@kx.tool → a governed stdio MCP tool).
    "LocalToolDef",
    "ToolError",
    "run",
    "invoke",
    "chat",
    "make_client",
    "default_client",
    "set_default_client",
    "TeamSummary",
    "TeamMember",
    "TeamMembers",
    "WarrantView",
    "GrantView",
    "AssetGrants",
    "DatasetSummary",
    "RetrievalMode",
    "DecayCandidate",
    "DecayReport",
    "Memory",
    "MemoryHit",
    "MemoryKind",
    "MemoryStats",
    "StoreResult",
    "ContentItem",
    "PutResult",
    "ContextBundle",
    "ContextBundleItem",
    "PutContextBundleResult",
    "Branch",
    "BranchItem",
    "CreateBranchResult",
    "SnapshotResult",
    "AdvanceResult",
    "EditProposal",
    "ModelSummary",
    "PullStatus",
    # POC-3 Models lifecycle: the load/offload outcome
    "ModelLifecycleResult",
    # POC-1 Settings: the connected gateway's effective config (display-only, SN-8)
    "ServerInfo",
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
    "CallToolResult",
    # D114 HITL approval gate + M11 cost-spend (ListPendingApprovals / Grant / Deny / GetRunCost)
    "PendingApproval",
    "PendingApprovalsPage",
    "RunCost",
    # RC1 (D172) agentic-evaluation per-run quality readout (ScoreRun)
    "RunScore",
    # D170 / MM-3 operator secret store (PutSecret / ListSecretNames / DeleteSecret)
    "SecretName",
    "SecretNamesPage",
    # D170 / D113 trigger admin (Register / List / Deregister / Submit / Test)
    "TriggerView",
    "TriggersPage",
    "trigger_kind_to_proto",
    "trigger_auth_to_proto",
    "trigger_kind_name",
    "trigger_auth_name",
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
