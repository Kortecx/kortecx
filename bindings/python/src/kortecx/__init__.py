"""kortecx — Python client SDK for the durable agentic-execution runtime.

A pure gRPC client over the frozen ``KxGateway`` contract. Import the clients and
go::

    from kortecx import KxClient

    with KxClient("http://127.0.0.1:50151") as kx:
        result = kx.invoke("kx/recipes/echo", {"topic": "hello"}, wait=True)
        print(result.text)
"""

from __future__ import annotations

from .blueprints import BlueprintBuilder, EdgeInput, StepInput
from .capture import CaptureRecord, CaptureRecordPage
from .client import DEFAULT_ENDPOINT, AsyncKxClient, KxClient
from .content import ContentItem, PutResult
from .datasets import DatasetHit, DatasetSummary, IngestDocument, IngestResult
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
from .grants import AssetGrants, GrantView
from .models import ModelSummary
from .motes import MoteConfigItem, MoteDetail, effect_pattern_name, nd_class_name
from .react import ReactTurn, ReactTurnPage
from .recipes import (
    BlueprintForm,
    BlueprintFormField,
    RecipeForm,
    RecipeFormField,
    blueprint_param_type_name,
    recipe_param_type_name,
)
from .replan import ReplanRound, ReplanRoundPage
from .run import AsyncRun, Result, Run
from .runs import RunPage, RunSummary
from .teams import TeamMember, TeamMembers, TeamSummary, WarrantView
from .toolscout import (
    BundleScore,
    BundleSpec,
    BundleTool,
    KeywordSet,
    ManifestScore,
    ToolManifest,
    lower_verdict_name,
)
from .types import Delta, Frame, MoteView, Projection, SignatureSummary, state_name
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
    "Delta",
    "Frame",
    "SignatureSummary",
    "RunSummary",
    "RunPage",
    "ReactTurn",
    "ReactTurnPage",
    "ReplanRound",
    "ReplanRoundPage",
    "CaptureRecord",
    "CaptureRecordPage",
    "RecipeForm",
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
    "TeamSummary",
    "TeamMember",
    "TeamMembers",
    "WarrantView",
    "GrantView",
    "AssetGrants",
    "DatasetSummary",
    "ContentItem",
    "PutResult",
    "ModelSummary",
    # Batch B: per-mote definition inspection (display-only)
    "MoteDetail",
    "MoteConfigItem",
    "nd_class_name",
    "effect_pattern_name",
    "DatasetHit",
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
    "state_name",
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
