"""kortecx — Python client SDK for the durable agentic-execution runtime.

A pure gRPC client over the frozen ``KxGateway`` contract. Import the clients and
go::

    from kortecx import KxClient

    with KxClient("http://127.0.0.1:50151") as kx:
        result = kx.invoke("kx/recipes/echo", {"topic": "hello"}, wait=True)
        print(result.text)
"""

from __future__ import annotations

from .client import DEFAULT_ENDPOINT, AsyncKxClient, KxClient
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
from .recipes import RecipeForm, RecipeFormField, recipe_param_type_name
from .run import AsyncRun, Result, Run
from .runs import RunPage, RunSummary
from .teams import TeamMember, TeamMembers, TeamSummary, WarrantView
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
    "RecipeForm",
    "RecipeFormField",
    "recipe_param_type_name",
    "TeamSummary",
    "TeamMember",
    "TeamMembers",
    "WarrantView",
    "GrantView",
    "AssetGrants",
    "DatasetSummary",
    "DatasetHit",
    "IngestResult",
    "IngestDocument",
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
