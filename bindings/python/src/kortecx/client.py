"""The kortecx gateway clients — :class:`KxClient` (sync) and :class:`AsyncKxClient`.

Thin, typed wrappers over the generated ``KxGatewayStub``: credential resolution,
channel lifecycle, and the eight RPCs, plus the high-level ``invoke(..., wait=True)``
"runtime as a function". Identity is server-derived (SN-8): the client sends a
*credential* (a bearer token), never a claimed identity, and never computes an id.
"""

from __future__ import annotations

import dataclasses as _dataclasses
import json
import os
import warnings
from typing import (
    TYPE_CHECKING,
    Any,
    AsyncIterator,
    Dict,
    Iterator,
    List,
    Mapping,
    Optional,
    Sequence,
    Tuple,
    Union,
    cast,
)

import grpc

if TYPE_CHECKING:
    from . import chains as _chains

from . import events as _events
from . import hexids, types
from . import wait as _wait  # aliased: `wait` is also a public kwarg name
from .alerts import AlertsPage, AlertSummary
from .approvals import PendingApproval, PendingApprovalsPage
from .apps import (
    AppSummary,
    SaveAppResult,
    ScaffoldLaunch,
    ScaffoldStatus,
    StoredApp,
    canonical_json,
)
from .apps import default_handle as _default_app_handle
from .branch import AdvanceResult, Branch, CreateBranchResult, EditProposal, SnapshotResult
from .capture import CaptureRecord, CaptureRecordPage
from .content import ContentItem, PutResult
from .context import ContextBundle, ContextBundleItem, PutContextBundleResult
from .cost import RunCost
from .datasets import (
    DatasetHit,
    DatasetSummary,
    FuzzyHit,
    IngestDocument,
    IngestResult,
    RetrievalMode,
    _to_documents,
)
from .errors import KxError, KxFailedPrecondition, KxUsage, from_rpc_error
from .eval import RunScore
from .feedback import FeedbackPage, FeedbackRow, rating_to_proto
from .grants import AssetGrants
from .memory import Memory, MemoryHit, MemoryKind, StoreResult
from .models import ModelLifecycleResult, ModelSummary, PullStatus
from .motes import MoteDetail
from .react import ReactTurn, ReactTurnPage
from .recipes import RecipeForm, RecipeInfo, ScoredRecipe
from .replan import ReplanRound, ReplanRoundPage
from .rerank import ReRankTurn, ReRankTurnPage
from .run import AsyncRun, Result, Run
from .runs import RunInputs, RunPage, RunSummary
from .secrets import SecretName, SecretNamesPage
from .server_info import ServerInfo
from .teams import TeamMembers, TeamSummary
from .telemetry import MoteTelemetryRow, TelemetryPage, TelemetrySummary
from .toolscout import (
    BundleScore,
    BundleSpec,
    CallToolResult,
    McpServer,
    McpServersPage,
    RegisteredTool,
    RegisteredToolsPage,
    RegisterServerResult,
    ToolManifest,
    ToolParam,
)
from .triggers import (
    TriggersPage,
    TriggerView,
    trigger_auth_to_proto,
    trigger_kind_to_proto,
)
from .v1 import gateway_pb2 as _g
from .v1 import gateway_pb2_grpc as _gg

#: The conventional gateway endpoint (matches ``kx serve`` / the CLI default).
DEFAULT_ENDPOINT = "http://127.0.0.1:50151"

#: Batch A: the env var a client reads for its default model when none is passed.
DEFAULT_MODEL_ENV = "KX_DEFAULT_MODEL"


def _fill_default_model(
    request: "_g.SubmitWorkflowRequest", default_model: str
) -> "_g.SubmitWorkflowRequest":
    """Batch A: fill any MODEL step that left ``model_id`` empty with the client's
    ``default_model``, in place, just before submit. A no-op when ``default_model`` is
    unset OR no step omitted its model — so the canonical lowering (which the corpus
    pins, client-free) is untouched and the server still binds ``""`` → the served
    model (SN-8) when neither is set. Returns ``request`` for chaining."""
    if not default_model:
        return request
    model_kind = _g.WorkflowStepKind.WORKFLOW_STEP_KIND_MODEL
    for step in request.steps:
        if step.kind == model_kind and not step.model_id:
            step.model_id = default_model
    return request


def _is_model_step(step: dict) -> bool:
    """True when a portable-blueprint step is a MODEL step (mirrors the CLI
    ``resolve_kind`` inference: an explicit ``kind``, else model fields ⇒ model)."""
    kind = step.get("kind")
    if kind is not None:
        return kind == "model"
    return bool(step.get("model_id") or step.get("prompt"))


def _inject_app_args(blueprint: dict, args: Optional[Dict[str, str]]) -> dict:
    """POC-5d: fold an App's ``input_schema`` args into the ENTRY (first) model step's
    prompt as a clearly-delimited "Inputs" block, returning a NEW blueprint (never
    mutates the source). A NO-OP when ``args`` is empty/absent OR the blueprint has no
    model step ⇒ byte-identical to the pre-POC-5d compile. The server still re-resolves
    every warrant from the caller's grants (SN-8); args steer, never grant."""
    entries = [(k, v) for k, v in (args or {}).items() if v is not None]
    if not entries:
        return blueprint
    steps = blueprint.get("steps", [])
    idx = next((i for i, s in enumerate(steps) if _is_model_step(s)), -1)
    if idx < 0:
        return blueprint
    block = "\n".join(f"- {k}: {v}" for k, v in entries)
    new_steps = list(steps)
    target = dict(new_steps[idx])
    prompt = target.get("prompt", "")
    target["prompt"] = f"{prompt}\n\nInputs:\n{block}".strip()
    new_steps[idx] = target
    return {**blueprint, "steps": new_steps}


#: The canonical ReAct recipe handle. A react run has NO statically-known terminal
#: Mote (the gateway hands back a run-salted turn-0 id that never commits, and the
#: settled Answer turn isn't known until the model emits it), so ``invoke(wait=True)``
#: on this handle waits on chain SETTLEMENT via ``ListReactTurns`` instead of a
#: single terminal Mote (campaign finding F13).
REACT_RECIPE_HANDLE = "kx/recipes/react"

#: POC-1 chat recipe handles. ``chat`` takes ``{"prompt": <text>}``; the AUTO-RAG
#: ``chat-rag`` adds ``{"dataset": <name>, "k": <int>}`` — the server embeds the
#: prompt, retrieves the dataset's top-k docs, folds them into the prompt, and
#: answers (HONESTLY degrading to a plain answer when the dataset is missing/empty,
#: never faking grounding). Both settle on a single terminal model Mote.
CHAT_RECIPE_HANDLE = "kx/recipes/chat"
CHAT_RAG_RECIPE_HANDLE = "kx/recipes/chat-rag"

#: PR-B2: the vision recipe handle — an image→text chat over a vision-capable model
#: on either engine (Ollama vision tags / llama.cpp mmproj). Also serves prompted OCR
#: ("transcribe the text in this image") — the same vision dispatch.
VISION_RECIPE_HANDLE = "kx/recipes/vision"

#: AGENTIC-VISION: the image-grounded ReAct recipe — the live agent loop PLUS a bound
#: image the served VLM reasons over on EVERY turn (the coordinator anchors it on the
#: turn-0 ReactRound + re-derives it edge-free for successor turns). Shares the
#: ``kx/recipes/react`` prefix, so :func:`_is_react_handle` settles it as a chain.
REACT_VISION_RECIPE_HANDLE = "kx/recipes/react-vision"

#: RC4b AGENTIC RAG: the dataset-grounded ReAct recipe — a live agent loop whose warrant
#: grants the read-only ``retrieve`` tool, so the model AUTONOMOUSLY searches a corpus
#: (hybrid keyword+semantic), reads the passages, and can re-query across turns. Invoke it
#: with ``{"instruction": <goal>, "dataset": <name>}``; the server folds the dataset name
#: into the instruction. Shares the ``kx/recipes/react`` prefix, so it settles as a chain.
REACT_RAG_RECIPE_HANDLE = "kx/recipes/react-rag"

#: RC4b VISION-RAG: a single grounded multimodal completion — the served VLM answers about
#: an attached image WHILE grounded on a dataset's top-k retrieved TEXT passages (one
#: generation). Bind ``{"prompt", "image_ref", "model", "dataset", "k"}`` (``chat(image=,
#: dataset=)`` does this). Datasets stay text-only (image-embedding is post-RC).
VISION_RAG_RECIPE_HANDLE = "kx/recipes/vision-rag"

ArgsType = Union[dict, bytes, bytearray, str]
IdType = Union[str, bytes]
#: PR-B2: an image to attach to :meth:`chat`. Raw ``bytes`` (uploaded), or a dict
#: ``{"ref": <64-hex>}`` (an existing ``PutContent`` ref) / ``{"bytes": ...,
#: "media_type": ...}``.
ImageInput = Union[bytes, bytearray, Dict[str, Any]]

#: Default channel message-size options (Batch A): receive covers large committed
#: results + full content batches; send covers a default-cap (32 MiB) PutContent
#: with headroom. Appended BEFORE user ``channel_options`` so an explicit user
#: value for the same key wins (gRPC takes the last occurrence).
_DEFAULT_CHANNEL_OPTIONS = [
    ("grpc.max_receive_message_length", 0x40000000),  # 1 GiB
    ("grpc.max_send_message_length", 0x04000000),  # 64 MiB
]


def _merged_options(channel_options: Optional[list]) -> list:
    return _DEFAULT_CHANNEL_OPTIONS + (channel_options or [])


def _is_react_handle(handle: str) -> bool:
    """True for a ReAct CHAIN recipe (``react`` / ``react-fs`` / ``react-auto``) —
    its ``invoke(wait=True)`` settles via the chain (``ListReactTurns``), not a
    terminal Mote (F13). They share the ``kx/recipes/react`` prefix. ``react-edit``
    is EXCLUDED: it is a single model step that settles on its terminal mote."""
    return handle.startswith(REACT_RECIPE_HANDLE) and handle != "kx/recipes/react-edit"


# --- shared credential + channel helpers -------------------------------------


def _is_nonloopback_plaintext(endpoint: str) -> bool:
    """True iff a bearer token would cross plaintext ``http://`` to a non-loopback
    host (mirrors the CLI's ``is_nonloopback_plaintext``)."""
    if not endpoint.startswith("http://"):
        return False
    rest = endpoint[len("http://") :]
    if rest.startswith("["):  # bracketed IPv6 host
        host = rest[1:].split("]", 1)[0]
    else:
        host = rest.split("/", 1)[0].split(":", 1)[0]
    return host not in ("127.0.0.1", "::1", "localhost")


def _resolve_token(endpoint: str, token: Optional[str], token_file: Optional[str]) -> Optional[str]:
    if token is not None and token_file is not None:
        raise KxUsage("token and token_file are mutually exclusive")
    resolved: Optional[str]
    if token_file is not None:
        with open(token_file, encoding="utf-8") as fh:
            resolved = fh.read().strip()
        if not resolved:
            raise KxUsage(f"token_file {token_file} is empty")
    elif token is not None:
        resolved = token
    else:
        env = os.environ.get("KX_TOKEN")
        resolved = env.strip() if env else None
    if resolved and _is_nonloopback_plaintext(endpoint):
        warnings.warn(
            f"sending a bearer token to a non-loopback plaintext endpoint ({endpoint}); "
            "it travels in cleartext — use an https:// endpoint (TLS) for remote use",
            stacklevel=3,
        )
    return resolved


def _target(endpoint: str) -> str:
    for scheme in ("http://", "https://"):
        if endpoint.startswith(scheme):
            return endpoint[len(scheme) :].rstrip("/")
    return endpoint.rstrip("/")


def _encode_args(args: ArgsType) -> bytes:
    """Coerce dict/str/bytes args to JSON bytes, failing fast on invalid JSON."""
    if isinstance(args, (bytes, bytearray)):
        raw = bytes(args)
    elif isinstance(args, str):
        raw = args.encode("utf-8")
    elif isinstance(args, dict):
        return json.dumps(args, separators=(",", ":")).encode("utf-8")
    else:
        raise KxUsage(f"args must be a dict, str, or bytes, got {type(args).__name__}")
    try:
        json.loads(raw)  # client-side sanity check (server is the fail-closed authority)
    except json.JSONDecodeError as e:
        raise KxUsage(f"args are not valid JSON: {e}") from e
    return raw


# --- sync client -------------------------------------------------------------


class _Connections:
    """The ``kx.connections`` namespace — connector (external MCP server) admin, with
    a verb vocabulary matching the ``kx connections`` CLI (``add`` / ``list`` /
    ``test`` / ``remove`` / ``discover``). Each method delegates 1:1 to the flat
    ``register_mcp_server`` / ``list_mcp_servers`` / ``test_mcp_server`` /
    ``deregister_mcp_server`` / ``discover_server_tools`` methods (which remain for
    back-compat). A connector = an external MCP tool server (see ``kx-extension-sdk``)."""

    def __init__(self, client: "KxClient") -> None:
        self._c = client

    def add(
        self,
        name: str,
        *,
        transport: str = "stdio",
        endpoint: str,
        args: Optional[Sequence[str]] = None,
        tls_required: bool = False,
        credential_ref: str = "",
        session_mode: str = "stateless",
    ) -> "RegisterServerResult":
        """Register (dial + discover) a connector. See ``register_mcp_server``."""
        return self._c.register_mcp_server(
            name=name,
            transport=transport,
            endpoint=endpoint,
            args=args,
            tls_required=tls_required,
            credential_ref=credential_ref,
            session_mode=session_mode,
        )

    def list(self, *, limit: int = 0, after_name: str = "") -> "McpServersPage":
        """List registered connectors + health. See ``list_mcp_servers``."""
        return self._c.list_mcp_servers(limit=limit, after_name=after_name)

    def test(self, name: str) -> bool:
        """Test a connector's reachability. See ``test_mcp_server``."""
        return self._c.test_mcp_server(name=name)

    def remove(self, name: str) -> bool:
        """Remove a connector + its tools. See ``deregister_mcp_server``."""
        return self._c.deregister_mcp_server(name=name)

    def discover(self, name: str) -> "RegisteredToolsPage":
        """Re-dial + re-discover a connector's tools. See ``discover_server_tools``."""
        return self._c.discover_server_tools(name=name)

    def fire(self, name: str, tool: str, args: Optional[str] = None) -> "CallToolResult":
        """Operator diagnostic: fire ONE registered tool live. See ``call_mcp_tool``."""
        return self._c.call_mcp_tool(name=name, tool=tool, args=args)


class _Memory:
    """The ``kx.memory`` namespace — durable agentic MEMORY (RC5a), with a verb
    vocabulary matching the ``kx memory`` CLI (``store`` / ``list`` / ``recall`` /
    ``forget``). Each method delegates 1:1 to the flat ``store_memory`` /
    ``list_memories`` / ``recall_memory`` / ``forget_memory`` methods. Every memory
    is scoped to the caller's own principal (server-derived)."""

    def __init__(self, client: "KxClient") -> None:
        self._c = client

    def store(
        self, content: "str | bytes", *, kind: MemoryKind = MemoryKind.SEMANTIC
    ) -> StoreResult:
        """Remember a fact (content-addressed, idempotent). See ``store_memory``."""
        return self._c.store_memory(content, kind=kind)

    def list(self, *, instance_id: Optional[str] = None, limit: int = 0) -> List[Memory]:
        """The episodic log, newest-first. See ``list_memories``."""
        return self._c.list_memories(instance_id=instance_id, limit=limit)

    def recall(self, text: str, *, k: int = 5) -> List[MemoryHit]:
        """The top-k most-similar memories. See ``recall_memory``."""
        return self._c.recall_memory(text, k=k)

    def forget(self, memory_id: str) -> bool:
        """Erase a memory by its content id (hex). See ``forget_memory``."""
        return self._c.forget_memory(memory_id)


class _Secrets:
    """The ``kx.secrets`` namespace — operator secret-store admin (D170 / MM-3),
    with a verb vocabulary mirroring ``kx.connections`` (``set`` / ``list`` /
    ``remove``). Each method delegates 1:1 to the flat ``put_secret`` /
    ``list_secret_names`` / ``delete_secret`` methods (which remain for
    back-compat). A secret holds a connector credential / trigger auth VALUE
    server-side; the value never returns over the wire (D81) — a
    ``credential_ref`` / ``auth_secret_ref`` NAMES one of these rows."""

    def __init__(self, client: "KxClient") -> None:
        self._c = client

    def set(self, name: str, value: str) -> bool:
        """Store (create or overwrite) a named secret. See ``put_secret``."""
        return self._c.put_secret(name=name, value=value)

    def list(self, *, limit: int = 0, after_name: str = "") -> "SecretNamesPage":
        """List the stored secret names + audit timestamps. See
        ``list_secret_names``."""
        return self._c.list_secret_names(limit=limit, after_name=after_name)

    def remove(self, name: str) -> bool:
        """Remove a named secret. See ``delete_secret``."""
        return self._c.delete_secret(name=name)

    def delete(self, name: str) -> bool:
        """Alias for :meth:`remove`."""
        return self.remove(name)


class _Triggers:
    """The ``kx.triggers`` namespace — durable webhook / cron / gRPC trigger admin
    (D170 / D113), with a verb vocabulary mirroring ``kx.connections`` (``add`` /
    ``list`` / ``test`` / ``fire`` / ``remove``). Each method delegates 1:1 to the
    flat ``register_trigger`` / ``list_triggers`` / ``test_trigger`` /
    ``submit_trigger`` / ``deregister_trigger`` methods (which remain for
    back-compat). ``kind`` / ``auth`` are friendly strings (an unknown one raises
    ``ValueError``). A trigger binds an inbound event to a published recipe."""

    def __init__(self, client: "KxClient") -> None:
        self._c = client

    def add(
        self,
        name: str,
        *,
        kind: str = "webhook",
        recipe: str = "",
        auth: str = "none",
        secret_ref: str = "",
        schedule: str = "",
        enabled: bool = True,
    ) -> str:
        """Register a trigger; returns the hex ``trigger_id``. See
        ``register_trigger``."""
        return self._c.register_trigger(
            name=name,
            kind=kind,
            recipe_handle=recipe,
            auth=auth,
            auth_secret_ref=secret_ref,
            schedule_spec=schedule,
            enabled=enabled,
        )

    def list(self, *, limit: int = 0, after_name: str = "") -> "TriggersPage":
        """List the registered triggers. See ``list_triggers``."""
        return self._c.list_triggers(limit=limit, after_name=after_name)

    def test(self, name: str, payload: str = "") -> "tuple[bool, str]":
        """Dry-run a trigger's binding without submitting a run — returns
        ``(ok, detail)``. See ``test_trigger``."""
        return self._c.test_trigger(name=name, payload_json=payload)

    def fire(self, name: str, payload: str = "", idempotency_key: str = "") -> "tuple[str, bool]":
        """Fire a trigger by name — returns ``(instance_id_hex, deduped)``. See
        ``submit_trigger``."""
        return self._c.submit_trigger(
            name=name, idempotency_key=idempotency_key, payload_json=payload
        )

    def remove(self, name: str) -> bool:
        """Remove a registered trigger. See ``deregister_trigger``."""
        return self._c.deregister_trigger(name=name)

    def delete(self, name: str) -> bool:
        """Alias for :meth:`remove`."""
        return self.remove(name)


class _Approvals:
    """The ``kx.approvals`` namespace — the HITL pre-action approval gate's operator
    control plane (D114): ``list_pending`` / ``grant`` / ``deny``. Grant/deny are
    OPERATOR decisions over a server-derived ``request_id`` — they release/reject a
    STAGED world-mutating action, never mint a client warrant (SN-8)."""

    def __init__(self, client: "KxClient") -> None:
        self._c = client

    def list_pending(self, *, limit: int = 0) -> "PendingApprovalsPage":
        """List the world-mutating actions withheld awaiting approval."""
        return self._c.list_pending_approvals(limit=limit)

    def grant(self, request_id: str, *, reason: str = "") -> bool:
        """Grant a pending approval (releases the staged action to fire exactly once).
        Returns ``True`` iff a decision was recorded (``False`` ⇒ unknown/resolved)."""
        return self._c.grant_approval(request_id=request_id, reason=reason)

    def deny(self, request_id: str, *, reason: str = "") -> bool:
        """Deny a pending approval (the gated chain dead-letters fail-closed)."""
        return self._c.deny_approval(request_id=request_id, reason=reason)


class _Cost:
    """The ``kx.cost`` namespace — the cost-spend guardrail readout (M11):
    ``get_run_cost``. A DISPLAY-ONLY local spend estimate, not Cloud billing."""

    def __init__(self, client: "KxClient") -> None:
        self._c = client

    def get_run_cost(self, instance_id: str) -> "RunCost":
        """The run's local spend estimate (priced turn/tool counters)."""
        return self._c.get_run_cost(instance_id=instance_id)

    def get(self, instance_id: str) -> "RunCost":
        """Alias for :meth:`get_run_cost`."""
        return self.get_run_cost(instance_id)


class _Eval:
    """The ``kx.eval`` namespace — the per-run quality readout (RC1/D172):
    ``score_run``. An expectation-free trajectory summary; the golden-suite gate runs
    offline via the ``kx eval run`` CLI."""

    def __init__(self, client: "KxClient") -> None:
        self._c = client

    def score_run(self, instance_id: str) -> "RunScore":
        """A live run's expectation-free quality summary."""
        return self._c.score_run(instance_id=instance_id)

    def score(self, instance_id: str) -> "RunScore":
        """Alias for :meth:`score_run`."""
        return self.score_run(instance_id)


class KxClient:
    """A synchronous client for a running ``kx serve`` gateway."""

    def __init__(
        self,
        endpoint: str = DEFAULT_ENDPOINT,
        *,
        token: Optional[str] = None,
        token_file: Optional[str] = None,
        default_model: str = "",
        channel_options: Optional[list] = None,
    ) -> None:
        self.endpoint = endpoint
        self._token = _resolve_token(endpoint, token, token_file)
        # Batch A: the default model to fill into MODEL steps that omit `model_id`
        # (a multi-model convenience; the server binds `""` → served when unset). An
        # explicit arg wins over the `KX_DEFAULT_MODEL` env fallback.
        self.default_model = default_model or os.environ.get(DEFAULT_MODEL_ENV, "")
        self._md = [("authorization", f"Bearer {self._token}")] if self._token else []
        if endpoint.startswith("https://"):
            self._channel = grpc.secure_channel(
                _target(endpoint),
                grpc.ssl_channel_credentials(),
                options=_merged_options(channel_options),
            )
        else:
            self._channel = grpc.insecure_channel(
                _target(endpoint), options=_merged_options(channel_options)
            )
        self._stub = _gg.KxGatewayStub(self._channel)

    # -- lifecycle --
    def close(self) -> None:
        self._channel.close()

    def __enter__(self) -> "KxClient":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def _call(self, fn):
        try:
            return fn()
        except grpc.RpcError as e:
            raise from_rpc_error(e) from e

    # -- RPCs --
    def invoke(
        self,
        handle: str,
        args: ArgsType,
        *,
        wait: bool = False,
        timeout: float = 120.0,
        wait_mode: str = "poll",
        out: Optional[str] = None,
        context: Optional[Sequence[str]] = None,
        context_refs: Optional[Sequence[str]] = None,
    ) -> Union[Run, Result]:
        """Bind a published recipe to ``args`` and run it.

        With ``wait=True`` blocks for the committed :class:`Result` (raising
        :class:`KxRunFailed` / :class:`KxWaitTimeout` on a failed / timed-out run);
        otherwise returns a :class:`Run` handle. ``wait_mode="events"`` uses the
        low-latency live subscription instead of polling.

        ``context`` is an optional list of context-bundle handles (PR-7) to attach;
        the server resolves each to its item refs and injects them into the entry
        Mote's IDENTITY-BEARING context, so a different context ⇒ a different run.
        ``context_refs`` (D155 Phase-3) attaches raw 64-hex content-store refs
        directly (no bundle) — same identity-bearing injection.
        """
        resp = self._call(
            lambda: self._stub.Invoke(
                _g.InvokeRequest(
                    handle=handle,
                    args=_encode_args(args),
                    context_bundles=list(context or []),
                    context_refs=list(context_refs or []),
                ),
                metadata=self._md,
            )
        )
        run = Run(self, resp.instance_id, resp.terminal_mote_id, resp.recipe_fingerprint)
        if not wait:
            return run
        if _is_react_handle(handle):
            # F13: a react chain settles via ListReactTurns, not a terminal Mote.
            # PR-R1: scope the settle poll to THIS invocation's chain (serve shares
            # one journal/instance_id across every Invoke) via react_chain_salt.
            outcome = _wait.poll_react_result(
                self._stub,
                self._md,
                resp.instance_id,
                resp.terminal_mote_id,
                timeout,
                resp.react_chain_salt,
            )
            result = _dataclasses.replace(
                self._finish(outcome),
                react_chain_salt=hexids.encode(resp.react_chain_salt)
                if resp.react_chain_salt
                else "",
            )
        else:
            result = self._await_terminal(
                resp.instance_id, resp.terminal_mote_id, timeout, wait_mode
            )
        if out is not None and result.payload is not None:
            with open(out, "wb") as fh:
                fh.write(result.payload)
        return result

    def _resolve_image_ref(self, image: ImageInput) -> str:
        """Resolve an :data:`ImageInput` to a 64-hex ``PutContent`` ref (PR-B2)."""
        if isinstance(image, (bytes, bytearray)):
            return self.put_content(bytes(image)).content_ref
        if isinstance(image, dict):
            if "ref" in image:
                return str(image["ref"])
            if "bytes" in image:
                return self.put_content(
                    bytes(image["bytes"]), media_type=image.get("media_type", "")
                ).content_ref
        raise KxUsage("image must be bytes, {'ref': <hex>}, or {'bytes': ..., 'media_type': ...}")

    def _bind_vision(self, prompt: str, image_ref: str) -> Tuple[str, dict]:
        """Bind ``kx/recipes/vision`` for an image-bearing chat (PR-B2), honest-degrading
        with a clear error when no image-capable model is served."""
        try:
            form = self.get_recipe_form(VISION_RECIPE_HANDLE)
        except Exception as e:  # recipe not provisioned / old gateway
            raise KxUsage(
                "vision is not available on this serve (no image-capable model). Pull/serve a "
                "vision model (e.g. gemma3 via Ollama, or Gemma-4 + mmproj via llama.cpp)."
            ) from e
        by = {f.name: f for f in form.fields}
        if "image_ref" not in by:
            raise KxUsage("the kx/recipes/vision form does not declare an image_ref slot")
        args: dict = {"image_ref": image_ref}
        if "prompt" in by:
            args["prompt"] = prompt
        model = by.get("model")
        if model is not None:
            args["model"] = (
                self.default_model
                if (self.default_model and self.default_model in model.allowed)
                else model.allowed[0]
            )
        return VISION_RECIPE_HANDLE, args

    def _bind_react_vision(self, args: dict, image_ref: str) -> Tuple[str, dict]:
        """AGENTIC-VISION: bind ``kx/recipes/react-vision`` (the image-grounded agent loop),
        injecting ``image_ref`` into the react args so the served VLM reasons over the image
        on every turn. Honest-degrades with a clear error when no vision model is served —
        never silently drops the image (GR15)."""
        try:
            form = self.get_recipe_form(REACT_VISION_RECIPE_HANDLE)
        except Exception as e:  # recipe not provisioned (text-only serve / old gateway)
            raise KxUsage(
                "agentic vision is not available on this serve (no image-capable model). "
                "Serve a vision model (e.g. gemma3 via Ollama, or Gemma-4 + mmproj via llama.cpp)."
            ) from e
        if "image_ref" not in {f.name for f in form.fields}:
            raise KxUsage("the kx/recipes/react-vision form does not declare an image_ref slot")
        return REACT_VISION_RECIPE_HANDLE, {**args, "image_ref": image_ref}

    def _bind_vision_rag(
        self, prompt: str, image_ref: str, dataset: str, k: int
    ) -> Tuple[str, dict]:
        """RC4b: bind ``kx/recipes/vision-rag`` — the VLM answers about the image WHILE
        grounded on the dataset's retrieved text (one generation). Honest-degrades with a
        clear error when vision-RAG is not provisioned (needs BOTH a vision model AND the
        dataset/hnsw features) — never silently drops the dataset (GR15)."""
        try:
            form = self.get_recipe_form(VISION_RAG_RECIPE_HANDLE)
        except Exception as e:  # recipe not provisioned (text-only / non-hnsw / old gateway)
            raise KxUsage(
                "vision-RAG is not available on this serve — it needs BOTH an image-capable "
                "model AND the dataset (hnsw) features. Drop 'dataset' for a plain vision "
                "answer, or serve a vision model with datasets enabled."
            ) from e
        by = {f.name: f for f in form.fields}
        if "image_ref" not in by:
            raise KxUsage("the kx/recipes/vision-rag form does not declare an image_ref slot")
        args: dict = {"image_ref": image_ref, "dataset": dataset, "k": k}
        if "prompt" in by:
            args["prompt"] = prompt
        model = by.get("model")
        if model is not None:
            args["model"] = (
                self.default_model
                if (self.default_model and self.default_model in model.allowed)
                else model.allowed[0]
            )
        return VISION_RAG_RECIPE_HANDLE, args

    def chat(
        self,
        prompt: str,
        *,
        dataset: Optional[str] = None,
        k: int = 4,
        timeout: float = 120.0,
        image: Optional[ImageInput] = None,
    ) -> str:
        """Ask the served model a single question and get its answer text (POC-1).

        A thin convenience over :meth:`invoke` + wait: it binds the published
        ``kx/recipes/chat`` recipe to ``{"prompt": prompt}`` and returns the
        committed answer string. When ``dataset`` is given it binds the AUTO-RAG
        ``kx/recipes/chat-rag`` recipe to ``{"prompt": prompt, "dataset": dataset,
        "k": k}`` instead — the server embeds the prompt, retrieves the dataset's
        top-``k`` documents, folds them into the prompt, and answers. If the
        dataset is missing/empty the server HONESTLY degrades to a plain answer
        (it never fakes grounding); the grounding refs stay server-side (SN-8), so
        this returns only the answer text.

        Raises :class:`~kortecx.errors.KxRunFailed` if the run fails and
        :class:`~kortecx.errors.KxWaitTimeout` if it does not commit in time — same
        as ``invoke(wait=True)``. ``chat-rag`` needs a gateway with the retrieval
        features (a recipe-not-found / unsupported run surfaces the usual error).

        PR-B2: pass ``image`` (raw ``bytes`` or ``{"ref": <hex>}``) to attach an image
        and bind ``kx/recipes/vision`` (image→text on a vision-capable model on either
        engine; also prompted OCR). RC4b: ``dataset`` + ``image`` together binds
        ``kx/recipes/vision-rag`` — the VLM answers about the image WHILE grounded on the
        dataset's retrieved text (a clear :class:`KxUsage` when vision-RAG is not
        provisioned, never a silent drop)."""
        if image is not None:
            image_ref = self._resolve_image_ref(image)
            if dataset is not None:
                v_handle, v_args = self._bind_vision_rag(prompt, image_ref, dataset, k)
            else:
                v_handle, v_args = self._bind_vision(prompt, image_ref)
            v_result = self.invoke(v_handle, v_args, wait=True, timeout=timeout)
            assert isinstance(v_result, Result)
            return v_result.text or ""
        if dataset is not None:
            handle = CHAT_RAG_RECIPE_HANDLE
            args: dict = {"prompt": prompt, "dataset": dataset, "k": k}
        else:
            handle = CHAT_RECIPE_HANDLE
            args = {"prompt": prompt}
        result = self.invoke(handle, args, wait=True, timeout=timeout)
        # invoke(wait=True) on a non-react handle always settles to a Result.
        assert isinstance(result, Result)  # narrow for mypy (never a Run here)
        return result.text or ""

    def submit_run(
        self, request: "_g.SubmitRunRequest", *, wait: bool = False, timeout: float = 120.0
    ) -> Union["_g.RunHandle", Result]:
        """Low-level propose-proxy submit (advanced; recipe authoring lives in the
        runtime). Returns the ``RunHandle``, or — with ``wait=True`` — the first
        committed :class:`Result`."""
        handle = self._call(lambda: self._stub.SubmitRun(request, metadata=self._md))
        if not wait:
            return handle
        outcome = _wait.poll_any(self._stub, self._md, handle.instance_id, timeout)
        return self._finish(outcome)

    def submit_workflow(
        self, request: "_g.SubmitWorkflowRequest", *, wait: bool = False, timeout: float = 120.0
    ) -> Union[Run, Result]:
        """Author a Tier-1 DAG (a :class:`BlueprintBuilder` ``build()``) and run it.
        The server COMPILES the DAG, derives all identity, and builds every warrant
        from the party's grants (SN-8) — the client sends only the topology + params.
        Returns a :class:`~kortecx.run.Run` handle (V2a — ``.wait()`` / ``.events()``),
        or — with ``wait=True`` — the first committed :class:`Result`. A workflow has no
        statically-known terminal, so the ``Run`` waits for the FIRST committed Mote. An
        old gateway without the seam raises ``KxUnimplemented``."""
        _fill_default_model(request, self.default_model)
        handle = self._call(lambda: self._stub.SubmitWorkflow(request, metadata=self._md))
        if not wait:
            return Run(self, handle.instance_id, b"", handle.recipe_fingerprint)
        outcome = _wait.poll_any(self._stub, self._md, handle.instance_id, timeout)
        return self._finish(outcome)

    def run_chain(
        self, chain: "_chains.Chain", *, wait: bool = False, timeout: float = 120.0
    ) -> Union[Run, Result]:
        """Lower a :class:`~kortecx.chains.Chain` (operator sugar or the string DSL)
        and run it. A thin convenience over :meth:`submit_workflow` — the server
        still COMPILES the DAG + builds every warrant from the party's grants
        (SN-8). Returns a :class:`~kortecx.run.Run` handle, or — with ``wait=True`` —
        the first committed :class:`Result`.

        V2b: any ``@kx.tool`` local functions referenced by the chain are registered
        (as stdio MCP servers the runtime dials) + resolved into their steps' tool
        contracts first; a chain with no local tools is unaffected."""
        from .tools import resolve_local_tools

        resolve_local_tools(self, chain)
        return self.submit_workflow(chain.build(), wait=wait, timeout=timeout)

    def get_projection(
        self, instance_id: IdType, *, at_seq: Optional[int] = None
    ) -> types.Projection:
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        req = _g.GetProjectionRequest(instance_id=inst)
        if at_seq is not None:
            req.at_seq = at_seq
        view = self._call(lambda: self._stub.GetProjection(req, metadata=self._md))
        return types.Projection.from_proto(view)

    def get_content(self, ref: IdType, instance_id: Optional[IdType] = None) -> bytes:
        """Fetch content by ref. With an ``instance_id`` (the run ownership
        ticket) it reads the run scope; ``None`` reads the UPLOADS scope (refs
        this party uploaded via :meth:`put_content`) — Batch A. Denials are
        uniform (no existence oracle)."""
        cref = hexids.as_bytes(ref, hexids.REF_LEN)
        inst = b"" if instance_id is None else hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        blob = self._call(
            lambda: self._stub.GetContent(
                _g.GetContentRequest(content_ref=cref, instance_id=inst), metadata=self._md
            )
        )
        return blob.payload

    def put_content(self, payload: bytes, *, media_type: str = "", filename: str = "") -> PutResult:
        """Upload bytes to the gateway's content store (Batch A). A CONTENT-STORE
        write, never a journal write: the returned ref is SERVER-DERIVED blake3
        (SN-8). ``media_type``/``filename`` are advisory audit fields. The server
        caps the payload fail-closed (``kx serve --content-max-bytes``, default
        32 MiB). An old gateway raises ``KxUnimplemented``."""
        resp = self._call(
            lambda: self._stub.PutContent(
                _g.PutContentRequest(payload=payload, media_type=media_type, filename=filename),
                metadata=self._md,
            )
        )
        return PutResult.from_proto(resp)

    def get_content_batch(
        self,
        refs: Sequence[IdType],
        *,
        instance_id: Optional[IdType] = None,
        max_bytes_per_item: Optional[int] = None,
    ) -> List[ContentItem]:
        """Fetch up to 64 refs in ONE round trip (Batch A — the N+1 collapse), in
        request order. ``instance_id`` scopes to a run; ``None`` reads the uploads
        scope. Unauthorized/missing/malformed refs come back as UNIFORM empty
        items (:attr:`ContentItem.missing`) — no existence oracle. Payloads
        truncate at ``min(max_bytes_per_item, the server's per-item clamp)`` with
        ``truncated`` set and ``full_size`` honest. More than 64 refs is refused."""
        content_refs = [hexids.as_bytes(r, hexids.REF_LEN) for r in refs]
        inst = b"" if instance_id is None else hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        req = _g.GetContentBatchRequest(instance_id=inst, content_refs=content_refs)
        if max_bytes_per_item is not None:
            req.max_bytes_per_item = max_bytes_per_item
        resp = self._call(lambda: self._stub.GetContentBatch(req, metadata=self._md))
        return [ContentItem.from_proto(i) for i in resp.items]

    def put_context_bundle(
        self,
        handle: str,
        items: Sequence[tuple],
        *,
        description: str = "",
    ) -> PutContextBundleResult:
        """Author (upsert) a context bundle (PR-7) at ``handle`` for this party.

        ``items`` is a sequence of ``(name, content_ref)`` or
        ``(name, content_ref, media_type)``; each ``content_ref`` is a ref already
        in the content store (e.g. from :meth:`put_content`). The server derives
        ``bundle_ref`` (SN-8) into an off-journal sidecar, scoped to this party.
        Attach the handle to a run with ``invoke(..., context=[handle])``. An old
        gateway raises ``KxUnimplemented``."""
        proto_items = []
        for it in items:
            name = str(it[0])
            cref = hexids.as_bytes(it[1], hexids.REF_LEN)
            media = str(it[2]) if len(it) > 2 else ""
            proto_items.append(_g.ContextItem(name=name, content_ref=cref, media_type=media))
        resp = self._call(
            lambda: self._stub.PutContextBundle(
                _g.PutContextBundleRequest(
                    handle=handle, description=description, items=proto_items
                ),
                metadata=self._md,
            )
        )
        return PutContextBundleResult.from_proto(resp)

    def list_context_bundles(self) -> List[ContextBundle]:
        """List this party's context bundles (PR-7) in handle order."""
        resp = self._call(
            lambda: self._stub.ListContextBundles(_g.ListContextBundlesRequest(), metadata=self._md)
        )
        return [ContextBundle.from_proto(b) for b in resp.bundles]

    def get_context_bundle(self, handle: str) -> Optional[ContextBundle]:
        """Fetch one context bundle by handle, or ``None`` if not found / not owned
        (uniform — no cross-party existence oracle)."""
        resp = self._call(
            lambda: self._stub.GetContextBundle(
                _g.GetContextBundleRequest(handle=handle), metadata=self._md
            )
        )
        return ContextBundle.from_proto(resp.bundle) if resp.found else None

    def delete_context_bundle(self, handle: str) -> bool:
        """Unbind a context bundle (its CAS blobs stay). Returns ``True`` iff a
        bundle was removed."""
        resp = self._call(
            lambda: self._stub.DeleteContextBundle(
                _g.DeleteContextBundleRequest(handle=handle), metadata=self._md
            )
        )
        return resp.removed

    # ----- POC-4 Apps (save / list / get / run; off-journal apps.db catalog) -----

    def save_app(
        self, envelope: "Mapping[str, object]", *, handle: Optional[str] = None
    ) -> SaveAppResult:
        """Persist a ``kortecx.app/v1`` envelope to the caller-scoped catalog. The
        server validates + canonicalizes it and derives ``app_ref`` (SN-8); the
        envelope carries NO authority. ``handle`` defaults to
        ``apps/local/<sanitized-name>``. An old gateway raises ``KxUnimplemented``."""
        h = handle or _default_app_handle(str(envelope.get("name", "app")))
        resp = self._call(
            lambda: self._stub.SaveApp(
                _g.SaveAppRequest(handle=h, envelope_json=canonical_json(envelope)),
                metadata=self._md,
            )
        )
        return SaveAppResult.from_proto(resp)

    def list_apps(self) -> List[AppSummary]:
        """List the caller's App catalog (deterministic handle order)."""
        resp = self._call(
            lambda: self._stub.ListApps(
                _g.ListAppsRequest(limit=0, after_handle=""), metadata=self._md
            )
        )
        return [AppSummary.from_proto(a) for a in resp.apps]

    def get_app(self, handle: str) -> Optional[StoredApp]:
        """Fetch one App by handle, or ``None`` if not found / not owned (uniform —
        no cross-party existence oracle)."""
        resp = self._call(
            lambda: self._stub.GetApp(_g.GetAppRequest(handle=handle), metadata=self._md)
        )
        return StoredApp.from_proto(resp) if resp.found else None

    def run_app(
        self,
        handle: str,
        *,
        args: Optional[Dict[str, str]] = None,
        wait: bool = False,
        timeout: float = 120.0,
    ) -> Union[Run, Result]:
        """Compile a saved App's blueprint and run it (exactly-once). Client-compose
        over ``GetApp`` → ``SubmitWorkflow`` — the server re-resolves EVERY warrant
        from the caller's grants (SN-8 / BLOCKER #5). Raises :class:`KxUsage` if the
        App is not found. POC-5d: ``args`` (the App's ``input_schema`` inputs) fold
        into the entry model step's prompt; empty/absent ⇒ byte-identical compile."""
        from .chains import Chain

        stored = self.get_app(handle)
        if stored is None:
            raise KxUsage(f"app {handle!r} not found")
        blueprint = _inject_app_args(stored.envelope["blueprint"], args)
        request = Chain.from_blueprint(blueprint)
        return self.submit_workflow(request, wait=wait, timeout=timeout)

    def get_app_structure(self, handle: str) -> Optional[dict]:
        """POC-5d: the App's portable blueprint (the agentic step structure the
        lineage editor renders/edits). A thin convenience over :meth:`get_app`
        (``envelope['blueprint']``); ``None`` for an absent/not-owned App (uniform —
        no existence oracle)."""
        stored = self.get_app(handle)
        return None if stored is None else stored.envelope["blueprint"]

    @staticmethod
    def _resolve_context_item(
        manifest: ContextBundle, item: "Union[str, int]"
    ) -> "tuple[int, ContextBundleItem]":
        """Resolve a context-item selector to ``(index, item)`` against ``manifest``.

        ``item`` is the advisory item NAME (a ``str``) or a 0-based INDEX (an
        ``int``). A name with more than one match is AMBIGUOUS — pass the index.
        Raises :class:`KxUsage` (a client-side selection error) on an out-of-range
        index, an unknown name, or an ambiguous name. ``bool`` is rejected (it is an
        ``int`` subtype, so ``True``/``False`` would silently mean index 1/0)."""
        items = manifest.items
        if isinstance(item, bool):
            raise KxUsage("item selector must be a name (str) or an index (int), not a bool")
        if isinstance(item, int):
            if item < 0 or item >= len(items):
                raise KxUsage(
                    f"item index {item} is out of range for bundle {manifest.handle!r} "
                    f"({len(items)} item{'s' if len(items) != 1 else ''})"
                )
            return item, items[item]
        matches = [(i, it) for i, it in enumerate(items) if it.name == item]
        if not matches:
            raise KxUsage(f"no item named {item!r} in bundle {manifest.handle!r}")
        if len(matches) > 1:
            raise KxUsage(
                f"item name {item!r} is ambiguous in bundle {manifest.handle!r} "
                f"({len(matches)} matches) — pass the integer index instead"
            )
        return matches[0]

    def _read_context_bundle_or_raise(
        self, handle: str, expect_bundle_ref: Optional[str]
    ) -> ContextBundle:
        """Re-read ``handle`` as the freshest edit base + run the optimistic-
        concurrency guard. With ``expect_bundle_ref`` set, a mismatch means the
        bundle changed under the caller ⇒ :class:`KxFailedPrecondition` (fail-closed,
        never a silent last-writer-wins clobber). The content-addressed
        ``bundle_ref`` is a free compare-and-swap token (any item/description change
        moves it)."""
        manifest = self.get_context_bundle(handle)
        if manifest is None:
            raise KxError(f"context bundle {handle!r} not found")
        if expect_bundle_ref is not None and manifest.bundle_ref != expect_bundle_ref:
            raise KxFailedPrecondition(
                f"context bundle {handle!r} changed since you read it "
                f"(expected bundle_ref {expect_bundle_ref}, now {manifest.bundle_ref}); "
                "re-read it and re-apply your change"
            )
        return manifest

    def edit_context_item(
        self,
        handle: str,
        item: "Union[str, int]",
        new_body: bytes,
        *,
        media_type: Optional[str] = None,
        expect_bundle_ref: Optional[str] = None,
    ) -> PutContextBundleResult:
        """Replace one context-item's body IN PLACE (POC-2 context-edit).

        The content store is IMMUTABLE, so this uploads ``new_body`` (a NEW
        server-derived ref via :meth:`put_content`) and re-upserts the bundle with
        that item re-pointed at the new ref — the item's advisory ``name`` and
        ``media_type`` are preserved unless ``media_type`` overrides. ``item``
        selects by name or index (see :meth:`_resolve_context_item`). Set
        ``expect_bundle_ref`` (the ``bundle_ref`` you viewed) to fail-closed on a
        concurrent change (:class:`KxFailedPrecondition`) instead of a silent
        overwrite. Editing to byte-identical content re-reports ``deduplicated``.
        Raises :class:`KxUsage` for an unknown/ambiguous item and :class:`KxError`
        if the bundle is gone."""
        manifest = self._read_context_bundle_or_raise(handle, expect_bundle_ref)
        idx, target = self._resolve_context_item(manifest, item)
        media = media_type if media_type is not None else target.media_type
        new_ref = self.put_content(new_body, media_type=media, filename=target.name).content_ref
        items = [(it.name, it.content_ref, it.media_type) for it in manifest.items]
        items[idx] = (target.name, new_ref, media)
        return self.put_context_bundle(handle, items, description=manifest.description)

    def remove_context_item(
        self,
        handle: str,
        item: "Union[str, int]",
        *,
        expect_bundle_ref: Optional[str] = None,
    ) -> PutContextBundleResult:
        """Drop one item from a bundle (POC-2) and re-upsert the remainder.

        Refuses (:class:`KxUsage`) if it would empty the bundle — the server rejects
        an empty manifest; use :meth:`delete_context_bundle` to unbind the whole
        handle. ``expect_bundle_ref`` makes it fail-closed on a concurrent change."""
        manifest = self._read_context_bundle_or_raise(handle, expect_bundle_ref)
        idx, _ = self._resolve_context_item(manifest, item)
        if len(manifest.items) <= 1:
            raise KxUsage(
                f"removing the last item would empty bundle {handle!r}; "
                "use delete_context_bundle to unbind the whole handle"
            )
        items = [
            (it.name, it.content_ref, it.media_type)
            for i, it in enumerate(manifest.items)
            if i != idx
        ]
        return self.put_context_bundle(handle, items, description=manifest.description)

    def export_context_item(self, handle: str, item: "Union[str, int]") -> bytes:
        """Fetch one context-item's FULL body bytes (POC-2) from the uploads scope.

        Returns the whole payload (the single :meth:`get_content` read is uncapped,
        unlike a preview-clamped batch fetch). Raises :class:`KxUsage` for an
        unknown/ambiguous item, :class:`KxError` if the bundle is gone, and the RPC's
        :class:`KxPermissionDenied` if the ref is not in this party's scope."""
        manifest = self.get_context_bundle(handle)
        if manifest is None:
            raise KxError(f"context bundle {handle!r} not found")
        _, target = self._resolve_context_item(manifest, item)
        return self.get_content(target.content_ref)

    def create_branch(
        self, handle: str, *, parent: str = "", description: str = ""
    ) -> CreateBranchResult:
        """Create (or fork via ``parent``) a D155 branch at ``handle`` for this party.

        A ``parent`` handle forks a point-in-time CoW sub-branch (it inherits the
        parent's resolved items at create time; later parent edits do not
        propagate). The server derives ``branch_ref`` (SN-8) into an off-journal
        sidecar, scoped to this party. An old gateway raises ``KxUnimplemented``."""
        resp = self._call(
            lambda: self._stub.CreateBranch(
                _g.CreateBranchRequest(
                    handle=handle, description=description, parent_handle=parent
                ),
                metadata=self._md,
            )
        )
        return CreateBranchResult.from_proto(resp)

    def snapshot_into(
        self,
        handle: str,
        paths: Sequence[str],
        *,
        parent: str = "",
        description: str = "",
    ) -> SnapshotResult:
        """Snapshot operator-approved host ``paths`` into the branch ``handle``.

        Each path is read (confined under ``KX_SERVE_FS_ROOT``, default-OFF) INTO
        the content store; the ``{path -> ref}`` manifest is recorded/merged. The
        branch is created (optionally from ``parent``) if absent. The host is never
        written (Phase-A). Raises ``KxFailedPrecondition`` when ``KX_SERVE_FS_ROOT``
        is unset, ``KxUnimplemented`` on an old gateway."""
        resp = self._call(
            lambda: self._stub.SnapshotInto(
                _g.SnapshotIntoRequest(
                    handle=handle,
                    paths=list(paths),
                    description=description,
                    parent_handle=parent,
                ),
                metadata=self._md,
            )
        )
        return SnapshotResult.from_proto(resp)

    def list_branches(self) -> List[Branch]:
        """List this party's D155 branches in handle order."""
        resp = self._call(
            lambda: self._stub.ListBranches(_g.ListBranchesRequest(), metadata=self._md)
        )
        return [Branch.from_proto(b) for b in resp.branches]

    def get_branch(self, handle: str) -> Optional[Branch]:
        """Fetch one branch's resolved manifest by handle, or ``None`` if not found
        / not owned (uniform — no cross-party existence oracle)."""
        resp = self._call(
            lambda: self._stub.GetBranch(_g.GetBranchRequest(handle=handle), metadata=self._md)
        )
        return Branch.from_proto(resp.branch) if resp.found else None

    def delete_branch(self, handle: str) -> bool:
        """Unbind a branch (its CAS blobs stay). Returns ``True`` iff one was removed."""
        resp = self._call(
            lambda: self._stub.DeleteBranch(
                _g.DeleteBranchRequest(handle=handle), metadata=self._md
            )
        )
        return resp.removed

    def advance_branch(self, handle: str, path: str, content_ref: IdType) -> AdvanceResult:
        """D155 Phase-3 (low-level): re-point ``path`` in branch ``handle`` to an
        EXISTING content-store ref (or insert it — "enrich"), then recompute
        ``branch_ref``. Strictly IN-CAS (no host read/write). Raises ``KxNotFound``
        if the branch is unknown, ``KxInvalidArgument`` if the ref does not resolve.
        Prefer :meth:`edit_branch` for the agentic high-level flow."""
        resp = self._call(
            lambda: self._stub.AdvanceBranch(
                _g.AdvanceBranchRequest(
                    handle=handle,
                    path=path,
                    content_ref=hexids.as_bytes(content_ref, hexids.REF_LEN),
                ),
                metadata=self._md,
            )
        )
        return AdvanceResult.from_proto(resp)

    def edit_branch_propose(
        self,
        handle: str,
        path: str,
        instruction: str,
        *,
        timeout: float = 300.0,
    ) -> EditProposal:
        """POC-5d: the PROPOSE half of :meth:`edit_branch` — run the
        ``kx/recipes/react-edit`` model step and return the proposed new body together
        with the file's current body, WITHOUT advancing the branch. The caller reviews
        the diff then either approves (``advance_branch(handle, path, result_ref)``) or
        rejects (discards — the proposed blob is a harmless content-addressed orphan).
        The host is NEVER written. Raises ``KxError`` if the step produced no committed
        answer or an empty body (GR15 fail-closed — same guards as :meth:`edit_branch`)."""
        branch = self.get_branch(handle)
        if branch is None:
            raise KxError(f"branch {handle!r} not found")
        item = next((it for it in branch.items if it.path == path), None)
        if item is None:
            raise KxError(f"path {path!r} is not in branch {handle!r}")
        directive = (
            f"You are editing the file `{path}`. The text in the attached context below IS its "
            f"exact current contents. Apply this change: {instruction}\n\nReturn ONLY the "
            "complete, updated file contents — no commentary, no explanation, and no markdown "
            "code fences."
        )
        # react-edit is a single model step; its only free param is `prompt`.
        result = self.invoke(
            "kx/recipes/react-edit",
            {"prompt": directive},
            wait=True,
            timeout=timeout,
            context_refs=[item.content_ref],
        )
        if not isinstance(result, Result) or not result.ok or result.result_ref is None:
            raise KxError("react-edit produced no committed answer to advance the branch to")
        # Fail CLOSED on an empty edit (GR15): never propose an empty file (a
        # heavy-reasoning model can return only stripped reasoning).
        if not result.payload:
            raise KxError(
                "react-edit produced an empty body (the model did not return file "
                "contents); the branch was NOT advanced"
            )
        current = self.get_branch_content(handle, path)
        return EditProposal(
            result_ref=result.result_ref,
            proposed_text=bytes(result.payload).decode("utf-8", "replace"),
            current_text=current.decode("utf-8", "replace") if current is not None else "",
        )

    def edit_branch(
        self,
        handle: str,
        path: str,
        instruction: str,
        *,
        timeout: float = 300.0,
    ) -> AdvanceResult:
        """D155 Phase-3: agentically edit a branch file IN-CAS in one shot. Runs the
        ``kx/recipes/react-edit`` model step and advances the manifest to the new
        content ref. The host is NEVER written. Raises ``KxError`` if the step produced
        no committed answer. (POC-5d: this is :meth:`edit_branch_propose` +
        :meth:`advance_branch` — the react-edit directive lives in exactly one place so
        the committed blob bytes are identical across both APIs.)"""
        proposal = self.edit_branch_propose(handle, path, instruction, timeout=timeout)
        return self.advance_branch(handle, path, proposal.result_ref)

    def scaffold_app(
        self,
        handle: str,
        *,
        goal: str = "",
        branch_handle: str = "",
    ) -> ScaffoldLaunch:
        """POC-5a: agentically scaffold an existing App's FIXED-skeleton project tree
        into its CoW branch (server-side; the host is never written). Returns
        immediately — poll :meth:`get_scaffold_status` (+ :meth:`get_branch`) for
        progress. The branch defaults to the App's own handle (one-App-one-branch)."""
        resp = self._call(
            lambda: self._stub.ScaffoldApp(
                _g.ScaffoldAppRequest(handle=handle, branch_handle=branch_handle, instruction=goal),
                metadata=self._md,
            )
        )
        return ScaffoldLaunch.from_proto(resp)

    def get_scaffold_status(self, branch_handle: str) -> ScaffoldStatus:
        """POC-5a: the live scaffold status for a branch (phase + done/pending files)."""
        resp = self._call(
            lambda: self._stub.GetScaffoldStatus(
                _g.GetScaffoldStatusRequest(branch_handle=branch_handle), metadata=self._md
            )
        )
        return ScaffoldStatus.from_proto(resp)

    def get_branch_content(self, handle: str, path: str) -> Optional[bytes]:
        """POC-5a: read one App project file's body THROUGH the caller's OWN branch
        manifest (caller-scoped). Returns ``None`` for an absent branch / absent path
        / not-owned (uniform — no existence oracle)."""
        resp = self._call(
            lambda: self._stub.GetBranchContent(
                _g.GetBranchContentRequest(handle=handle, path=path), metadata=self._md
            )
        )
        return bytes(resp.payload) if resp.found else None

    def lock_app(self, branch_handle: str) -> bool:
        """POC-5b: lock the App's project branch (agentic in-CAS edits are refused)."""
        resp = self._call(
            lambda: self._stub.LockApp(
                _g.LockAppRequest(branch_handle=branch_handle), metadata=self._md
            )
        )
        return resp.locked

    def unlock_app(self, branch_handle: str) -> bool:
        """POC-5b: unlock the App's project branch (re-enable agentic edits)."""
        resp = self._call(
            lambda: self._stub.UnlockApp(
                _g.UnlockAppRequest(branch_handle=branch_handle), metadata=self._md
            )
        )
        return resp.unlocked

    def list_models(self) -> List[ModelSummary]:
        """Discover the models the connected gateway serves (Batch A). Display
        only (SN-8): selection stays a recipe ENUM free-param validated
        server-side. Each entry reports live RAM residency (``loaded``) and the
        recipe ``chat_handle`` that routes a turn to it. An FFI-free gateway
        returns an EMPTY list; an old gateway raises ``KxUnimplemented``."""
        resp = self._call(lambda: self._stub.ListModels(_g.ListModelsRequest(), metadata=self._md))
        return [ModelSummary.from_proto(m) for m in resp.models]

    def load_model(self, model_id: str) -> ModelLifecycleResult:
        """POC-3: warm a REGISTERED local model into RAM (real load). An
        unregistered id raises ``KxNotFound`` (fail-closed — never an arbitrary
        path); an FFI-free gateway raises ``KxUnimplemented``. Over-capacity ⇒
        honest LRU-evict-oldest (sequential swap)."""
        resp = self._call(
            lambda: self._stub.LoadModel(_g.LoadModelRequest(model_id=model_id), metadata=self._md)
        )
        return ModelLifecycleResult.from_load(resp)

    def offload_model(self, model_id: str) -> ModelLifecycleResult:
        """POC-3: evict a REGISTERED local model from RAM (real
        ``llama_model_free``). Idempotent (``was_resident=False`` if it was not
        loaded); an unregistered id raises ``KxNotFound``."""
        resp = self._call(
            lambda: self._stub.OffloadModel(
                _g.OffloadModelRequest(model_id=model_id), metadata=self._md
            )
        )
        return ModelLifecycleResult.from_offload(resp)

    def get_server_info(self) -> ServerInfo:
        """The connected gateway's effective configuration (POC-1 Settings) — the
        served model, listen/bridge/console/metrics addresses, content/journal/
        catalog locations, the admission caps, the CORS allow-list, and the
        compiled-in feature flags. Authenticated like every other RPC; DISPLAY/
        SETTINGS-ONLY (SN-8) and NEVER a secret. An old gateway without the RPC
        raises ``KxUnimplemented``."""
        resp = self._call(
            lambda: self._stub.GetServerInfo(_g.GetServerInfoRequest(), metadata=self._md)
        )
        return ServerInfo.from_proto(resp)

    def pull_model(
        self,
        *,
        ollama_tag: Optional[str] = None,
        url: Optional[str] = None,
        sha256: Optional[str] = None,
    ) -> str:
        """Model Control v2: download + RUNTIME-register a model (no restart). Pass
        ``ollama_tag`` to pull from the Ollama registry, OR ``url`` (a
        ``huggingface.co`` ``/resolve/`` GGUF link) WITH ``sha256``. Returns the
        ``model_id`` to poll via :meth:`get_pull_status`. Deny-by-default: a refusal
        (downloads disabled / host not allowlisted / missing sha256) raises
        ``KxFailedPrecondition``. HOST INFRASTRUCTURE, not a client Mote (SN-8)."""
        if (ollama_tag is None) == (url is None):
            raise ValueError("pull_model requires exactly one of ollama_tag or url")
        req = _g.PullModelRequest(sha256=sha256 or "")
        if ollama_tag is not None:
            req.ollama_tag = ollama_tag
        else:
            req.url = url or ""
        resp = self._call(lambda: self._stub.PullModel(req, metadata=self._md))
        if not resp.accepted:
            from .errors import KxFailedPrecondition

            raise KxFailedPrecondition(f"pull refused: {resp.detail}")
        return resp.model_id

    def get_pull_status(self, model_id: str) -> PullStatus:
        """Model Control v2: the current progress of a :meth:`pull_model` download +
        registration (advisory). An unknown id raises ``KxNotFound``."""
        resp = self._call(
            lambda: self._stub.GetPullStatus(
                _g.GetPullStatusRequest(model_id=model_id), metadata=self._md
            )
        )
        return PullStatus.from_proto(model_id, resp)

    def set_active_model(self, model_id: str = "") -> str:
        """Model Control v2: set the server's ACTIVE default model (an off-journal
        advisory hint; the server never re-routes ``kx/recipes/chat``). An empty
        ``model_id`` CLEARS it (back to the primary). A non-served id raises
        ``KxNotFound``. Returns the active id after the op ("" ⇒ cleared)."""
        resp = self._call(
            lambda: self._stub.SetActiveModel(
                _g.SetActiveModelRequest(model_id=model_id), metadata=self._md
            )
        )
        return resp.active_model_id

    def stream_events(
        self, instance_id: IdType, *, since: int = 0, follow: bool = False
    ) -> Iterator[types.Delta]:
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        return _events.stream_deltas(self._stub, self._md, inst, since, follow)

    def ws_events(
        self, instance_id: IdType, *, since: int = 0, ws_endpoint: Optional[str] = None
    ) -> Iterator[types.Delta]:
        """Consume the live tail over the optional R5 WebSocket bridge (``kortecx[ws]``)."""
        inst_hex = hexids.encode(hexids.as_bytes(instance_id, hexids.INSTANCE_LEN))
        return _events.ws_stream_deltas(
            self.endpoint, inst_hex, since=since, token=self._token, ws_endpoint=ws_endpoint
        )

    def stream_model_tokens(
        self, instance_id: IdType, mote_id: IdType, *, since: int = 0
    ) -> Iterator[types.TokenChunk]:
        """Native gRPC ADVISORY token tail for ONE model mote (PR-4.2 / T-STREAM1):
        the NEW bytes per decode step until ``done``. ``mote_id`` must belong to
        ``instance_id``'s run (server-gated). The committed ``result_ref`` stays the
        authority — reconcile to it. An old gateway raises ``KxUnimplemented``."""
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        mote = hexids.as_bytes(mote_id, hexids.REF_LEN)
        return _events.stream_model_tokens(self._stub, self._md, inst, mote, since)

    def ws_tokens(
        self,
        instance_id: IdType,
        mote_id: IdType,
        *,
        since: int = 0,
        ws_endpoint: Optional[str] = None,
    ) -> Iterator[types.TokenChunk]:
        """Consume one model mote's ADVISORY token stream over the WS bridge (PR-4.2;
        ``kortecx[ws]``) — the browser/firewall-friendly path."""
        inst_hex = hexids.encode(hexids.as_bytes(instance_id, hexids.INSTANCE_LEN))
        mote_hex = hexids.encode(hexids.as_bytes(mote_id, hexids.REF_LEN))
        return _events.ws_stream_model_tokens(
            self.endpoint,
            inst_hex,
            mote_hex,
            since=since,
            token=self._token,
            ws_endpoint=ws_endpoint,
        )

    def stream_all_events(
        self, *, since: int = 0, follow: bool = False
    ) -> Iterator[types.GlobalDelta]:
        """The cross-run GLOBAL event tail (Batch C) — every run on the node over
        ONE stream, each delta stamped with its registration-watermark
        ``instance_id`` (display attribution, never identity) plus the
        ``run_registered`` kind the per-run cursor never surfaces.
        OPERATOR-GLOBAL on single-node OSS. Same cursor semantics as
        :meth:`stream_events`. An old gateway raises ``KxUnimplemented``."""
        return _events.stream_all_deltas(self._stub, self._md, since, follow)

    def ws_all_events(
        self, *, since: int = 0, ws_endpoint: Optional[str] = None
    ) -> Iterator[types.GlobalDelta]:
        """Consume the GLOBAL live tail over the optional WebSocket bridge
        (``kortecx[ws]``) — the ``/v1/events/all`` channel (Batch C)."""
        return _events.ws_stream_all_deltas(
            self.endpoint, since=since, token=self._token, ws_endpoint=ws_endpoint
        )

    def list_signatures(self) -> List[types.SignatureSummary]:
        resp = self._call(
            lambda: self._stub.ListSignatures(_g.ListSignaturesRequest(), metadata=self._md)
        )
        return [types.SignatureSummary.from_proto(s) for s in resp.signatures]

    def get_signature(self, signature_id: IdType) -> bytes:
        sid = hexids.as_bytes(signature_id, hexids.REF_LEN)
        resp = self._call(
            lambda: self._stub.GetSignature(
                _g.GetSignatureRequest(signature_id=sid), metadata=self._md
            )
        )
        return resp.manifest

    def register_signature(self, manifest: bytes) -> str:
        resp = self._call(
            lambda: self._stub.RegisterSignature(
                _g.RegisterSignatureRequest(manifest=manifest), metadata=self._md
            )
        )
        return hexids.encode(resp.signature_id)

    def list_runs(
        self, *, limit: Optional[int] = None, before_seq: Optional[int] = None
    ) -> RunPage:
        """Enumerate the durable registered runs (newest-first, paginated) — the
        "re-open by instance-id" primitive. ``before_seq`` resumes from the
        ``registered_seq`` of the last run seen; ``limit`` bounds the page."""
        req = _g.ListRunsRequest()
        if limit is not None:
            req.limit = limit
        if before_seq is not None:
            req.before_seq = before_seq
        resp = self._call(lambda: self._stub.ListRuns(req, metadata=self._md))
        return RunPage(runs=[RunSummary.from_proto(r) for r in resp.runs], has_more=resp.has_more)

    def get_run_inputs(self, instance_id: IdType) -> RunInputs:
        """The args a run was submitted with (PR-D ``GetRunInputs``) — the baseline
        for "Re-run with changes": fetch the captured args + handle, edit, then
        :meth:`invoke` again (only the changed sub-DAG recomputes). Useful when a
        run is recovered from :meth:`list_runs` with no client-side state. A run
        with nothing captured raises ``KxNotFound``; an old gateway without the
        sidecar raises ``KxUnimplemented``."""
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        resp = self._call(
            lambda: self._stub.GetRunInputs(
                _g.GetRunInputsRequest(instance_id=inst), metadata=self._md
            )
        )
        return RunInputs.from_proto(resp)

    def get_mote_detail(self, instance_id: IdType, mote_id: IdType) -> MoteDetail:
        """Resolve one Mote's admitted definition (Batch B) — the node-inspector
        read: step kind, model, prompt, capped params, tool contract. DISPLAY
        ONLY (SN-8). Commit-gated: an uncommitted mote (or one admitted by a
        pre-Batch-B binary) answers ``def_found=False`` honestly; an unknown
        mote in an owned run raises ``KxNotFound``; a wrong ticket raises the
        uniform ``KxPermissionDenied``. An old gateway raises ``KxUnimplemented``."""
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        mote = hexids.as_bytes(mote_id, hexids.REF_LEN)
        resp = self._call(
            lambda: self._stub.GetMoteDetail(
                _g.GetMoteDetailRequest(instance_id=inst, mote_id=mote), metadata=self._md
            )
        )
        return MoteDetail.from_proto(resp)

    def list_react_turns(
        self,
        *,
        instance_id: Optional[str] = None,
        step_salt: Optional[str] = None,
        limit: Optional[int] = None,
    ) -> ReactTurnPage:
        """Enumerate a live ReAct chain's durable turn facts (newest-first,
        paginated) — the queryable Reason→Act→Observe history. ``instance_id``
        (hex) scopes to one run; ``step_salt`` (hex 32B, PR-R1) further scopes to one
        CHAIN within it (serve's shared journal carries one chain per Invoke plus
        agentic-step chains) — pass ``Result.react_chain_salt`` from a react invoke.
        Absent enumerates every chain. The server clamps ``limit`` to its max page."""
        req = _g.ListReactTurnsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if step_salt:
            req.step_salt = hexids.decode(step_salt)
        if limit is not None:
            req.limit = limit
        resp = self._call(lambda: self._stub.ListReactTurns(req, metadata=self._md))
        return ReactTurnPage(
            turns=[ReactTurn.from_proto(t) for t in resp.turns], has_more=resp.has_more
        )

    def list_replan_rounds(self, *, limit: Optional[int] = None) -> ReplanRoundPage:
        """Enumerate a run's model-driven re-plan rounds (newest-first,
        paginated). The server clamps ``limit`` to its max page."""
        req = _g.ListReplanRoundsRequest()
        if limit is not None:
            req.limit = limit
        resp = self._call(lambda: self._stub.ListReplanRounds(req, metadata=self._md))
        return ReplanRoundPage(
            rounds=[ReplanRound.from_proto(r) for r in resp.rounds], has_more=resp.has_more
        )

    def list_rerank_turns(
        self, *, instance_id: Optional[str] = None, limit: Optional[int] = None
    ) -> ReRankTurnPage:
        """Enumerate a live listwise LLM re-rank loop's durable turn facts
        (newest-first, paginated; RC4c-2) — the queryable re-rank history with the
        enforced permutation per settled turn. ``instance_id`` (hex) scopes to one
        run; absent enumerates every run. The server clamps ``limit`` to its max
        page."""
        req = _g.ListReRankTurnsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if limit is not None:
            req.limit = limit
        resp = self._call(lambda: self._stub.ListReRankTurns(req, metadata=self._md))
        return ReRankTurnPage(
            turns=[ReRankTurn.from_proto(t) for t in resp.turns], has_more=resp.has_more
        )

    def list_capture_records(
        self, *, instance_id: Optional[str] = None, limit: Optional[int] = None
    ) -> CaptureRecordPage:
        """Enumerate the Morphic Data Engine's durably-captured ACTION records
        (newest-first, paginated) — the serve-path action exhaust. ``instance_id``
        (hex) scopes to one run; absent enumerates every captured action. The
        server clamps ``limit`` to its max page. An old gateway (or one without
        the capture sidecar) raises ``KxUnimplemented``."""
        req = _g.ListCaptureRecordsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if limit is not None:
            req.limit = limit
        resp = self._call(lambda: self._stub.ListCaptureRecords(req, metadata=self._md))
        return CaptureRecordPage(
            records=[CaptureRecord.from_proto(r) for r in resp.records], has_more=resp.has_more
        )

    def list_mote_telemetry(
        self,
        *,
        instance_id: Optional[str] = None,
        mote_id: Optional[str] = None,
        limit: Optional[int] = None,
        before_seq: Optional[int] = None,
    ) -> TelemetryPage:
        """Enumerate the host-recorded mote execution telemetry (newest-first,
        paginated) — wall-clock, model usage, the fired tool (Batch C). AUDIT/
        DISPLAY only: lives in a rebuildable-to-empty ``telemetry.db`` sidecar,
        never truth, never identity. ``instance_id``/``mote_id`` (hex) scope the
        page; ``before_seq`` resumes below the last row's seq. The server clamps
        ``limit`` to its max page. An old gateway (or one without the sidecar)
        raises ``KxUnimplemented``."""
        req = _g.ListMoteTelemetryRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if mote_id is not None:
            req.mote_id = hexids.decode(mote_id)
        if limit is not None:
            req.limit = limit
        if before_seq is not None:
            req.before_seq = before_seq
        resp = self._call(lambda: self._stub.ListMoteTelemetry(req, metadata=self._md))
        return TelemetryPage(
            rows=[MoteTelemetryRow.from_proto(r) for r in resp.rows], has_more=resp.has_more
        )

    def list_telemetry_summary(
        self,
        *,
        instance_id: Optional[str] = None,
    ) -> TelemetrySummary:
        """The EXACT, cross-page per-model token-economy rollup (W1a-3) — output
        tokens + wall-clock summed ``GROUP BY model_id`` server-side over the same
        ``telemetry.db`` sidecar, so a long ReAct run is summed honestly (unlike a
        client fold over the page-clamped :meth:`list_mote_telemetry`). Token-only,
        no cost/$ (billing is CLOUD). ``instance_id`` (hex) scopes to one run; absent
        sums all runs. An old gateway (or one without the sidecar) raises
        ``KxUnimplemented``."""
        req = _g.ListTelemetrySummaryRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        resp = self._call(lambda: self._stub.ListTelemetrySummary(req, metadata=self._md))
        return TelemetrySummary.from_proto(resp)

    def list_alerts(
        self,
        *,
        instance_id: Optional[str] = None,
        limit: Optional[int] = None,
        before_seq: Optional[int] = None,
    ) -> AlertsPage:
        """Enumerate the operator alerts inbox (newest-first, paginated) — the
        journal's TERMINAL ``Failed`` facts (dead-letters + worker-reported
        terminal failures) folded into a rebuildable-to-empty ``alerts.db``
        read-cache (W1a-2). DISPLAY/TRIAGE-READ only: never truth, never identity,
        never a digest input. ``instance_id`` (hex) scopes the page; ``before_seq``
        resumes below the last row's seq. The server clamps ``limit`` to its max
        page. The triage lifecycle (ack/resolve) is a Cloud capability (D156) — not
        exposed here. An old gateway (or one without the sidecar) raises
        ``KxUnimplemented``."""
        req = _g.ListAlertsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if limit is not None:
            req.limit = limit
        if before_seq is not None:
            req.before_seq = before_seq
        resp = self._call(lambda: self._stub.ListAlerts(req, metadata=self._md))
        return AlertsPage(
            alerts=[AlertSummary.from_proto(a) for a in resp.alerts], has_more=resp.has_more
        )

    def discover_tools(
        self,
        *,
        limit: int = 0,
        after_name: str = "",
        after_version: str = "",
    ) -> RegisteredToolsPage:
        """The durable tools registry INVENTORY (PR-6a ``DiscoverTools``) —
        registered tools + their authority/provenance, in ``(name, version)``
        order. DISTINCT from ``list_tool_manifests`` (advisory ranking).
        Registration grants NO authority (SN-8). An old gateway (or one without
        the registry) raises ``KxUnimplemented``."""
        req = _g.DiscoverToolsRequest(
            limit=limit, after_name=after_name, after_version=after_version
        )
        resp = self._call(lambda: self._stub.DiscoverTools(req, metadata=self._md))
        return RegisteredToolsPage(
            tools=[RegisteredTool.from_proto(t) for t in resp.tools], has_more=resp.has_more
        )

    def register_tool(
        self,
        *,
        name: str,
        version: str,
        server_host: str,
        description: str = "",
        idempotency_class: str = "Readback",
        remote_name: str = "",
        params: Optional[Sequence[ToolParam]] = None,
        deny_unknown_params: bool = True,
    ) -> str:
        """Register a declarative EXTERNAL MCP tool (PR-6a ``RegisterTool``). The
        server SSRF-vets ``server_host``, derives identity + capability, and
        durably stores it; the returned ``tool_id`` (hex) is SERVER-derived (the
        client never names/forges it, SN-8). Registration grants NO authority — a
        tool fires only under a server-issued warrant. DIALING ``server_host`` is a
        Cloud / PR-6b capability. An internal/link-local host is refused
        (``permission_denied``)."""
        schema = None
        if params:
            schema = _g.ToolInputSchema(
                params=[p.to_proto() for p in params], deny_unknown=deny_unknown_params
            )
        req = _g.RegisterToolRequest(
            tool_name=name,
            tool_version=version,
            description=description,
            idempotency_class=idempotency_class,
            input_schema=schema,
            server_host=server_host,
            remote_name=remote_name,
        )
        resp = self._call(lambda: self._stub.RegisterTool(req, metadata=self._md))
        return hexids.encode(resp.tool_id)

    def deregister_tool(self, *, name: str, version: str) -> bool:
        """Deregister an operator-registered tool by exact ``(name, version)``
        (PR-6a ``DeregisterTool``). Built-ins are refused (returns ``False``).
        Returns ``True`` iff a row was removed."""
        req = _g.DeregisterToolRequest(tool_name=name, tool_version=version)
        resp = self._call(lambda: self._stub.DeregisterTool(req, metadata=self._md))
        return resp.removed

    # --- PR-6b-1: the external MCP gateway (dial Py/TS-SDK-exposed MCP servers) ---

    def register_mcp_server(
        self,
        *,
        name: str,
        transport: str = "stdio",
        endpoint: str,
        args: Optional[Sequence[str]] = None,
        tls_required: bool = False,
        credential_ref: str = "",
        session_mode: str = "stateless",
    ) -> RegisterServerResult:
        """Register an EXTERNAL MCP server (PR-6b-1 ``RegisterMcpServer``) — the
        runtime DIALS it (``initialize`` → ``tools/list``) and registers its tools
        into the durable registry (each namespaced ``<name>/<remote>``).

        ``transport`` is ``"stdio"`` (``endpoint`` = the program path, ``args`` =
        its command line) or ``"http"`` (``endpoint`` = the URL; ``tls_required``
        refuses plaintext ``http://``). ``credential_ref`` names an env var / vault
        key (the secret VALUE is never sent, D81). The host is SSRF-vetted at
        admission AND at dial time; an internal host is refused
        (``permission_denied``). A dial failure is NOT fatal — the server persists
        with ``health="unreachable"`` (honest, never a fabricated success).

        ``session_mode`` is the firing posture (PR-6b-3): ``"stateless"`` (the
        default — a self-contained single-shot session per call, best for
        idempotent read tools and servers behind a round-robin load balancer) or
        ``"stateful"`` (one reused long-lived session, for servers that require it
        or chatty same-server traffic)."""
        req = _g.RegisterMcpServerRequest(
            server_name=name,
            transport=transport,
            endpoint=endpoint,
            args=list(args or []),
            tls_required=tls_required,
            credential_ref=credential_ref,
            session_mode=session_mode,
        )
        resp = self._call(lambda: self._stub.RegisterMcpServer(req, metadata=self._md))
        return RegisterServerResult(
            connection_id=hexids.encode(resp.connection_id),
            discovered=resp.discovered,
            health=resp.health,
        )

    def list_mcp_servers(self, *, limit: int = 0, after_name: str = "") -> McpServersPage:
        """List the registered external MCP servers + their health (PR-6b-1
        ``ListMcpServers``), in ``(name)`` order."""
        req = _g.ListMcpServersRequest(limit=limit, after_name=after_name)
        resp = self._call(lambda: self._stub.ListMcpServers(req, metadata=self._md))
        return McpServersPage(
            servers=[McpServer.from_proto(s) for s in resp.servers], has_more=resp.has_more
        )

    def discover_server_tools(self, *, name: str) -> RegisteredToolsPage:
        """Re-dial a registered server + re-discover its tools (PR-6b-1
        ``DiscoverServerTools``); returns the server's registered tools."""
        req = _g.DiscoverServerToolsRequest(server_name=name)
        resp = self._call(lambda: self._stub.DiscoverServerTools(req, metadata=self._md))
        # `discovered` is the count; the rows are the registered inventory.
        return RegisteredToolsPage(
            tools=[RegisteredTool.from_proto(t) for t in resp.tools], has_more=False
        )

    def test_mcp_server(self, *, name: str) -> bool:
        """Test a server's reachability — dial + ``initialize`` only (PR-6b-1
        ``TestMcpServer``). Returns ``True`` iff the handshake succeeded."""
        req = _g.TestMcpServerRequest(server_name=name)
        resp = self._call(lambda: self._stub.TestMcpServer(req, metadata=self._md))
        return resp.reachable

    def deregister_mcp_server(self, *, name: str) -> bool:
        """Remove a registered server + deregister its tools (PR-6b-1
        ``DeregisterMcpServer``). Returns ``True`` iff a server was removed."""
        req = _g.DeregisterMcpServerRequest(server_name=name)
        resp = self._call(lambda: self._stub.DeregisterMcpServer(req, metadata=self._md))
        return resp.removed

    def call_mcp_tool(self, *, name: str, tool: str, args: Optional[str] = None) -> CallToolResult:
        """Operator DIAGNOSTIC: fire ONE registered tool on a dialed connector live
        through the broker (``CallMcpTool``). ``args`` is a JSON object string
        (validated against the tool's inputSchema; ``None``/empty ⇒ ``{}``). NOT a
        durable agentic effect (no journal fact) — the "does this connector work"
        check; the agentic loop fires the same tools durably. SN-8 is re-enforced
        server-side (single-grant warrant from the tool's own scopes)."""
        req = _g.CallMcpToolRequest(
            server_name=name,
            remote_name=tool,
            args_json=args or "{}",
        )
        resp = self._call(lambda: self._stub.CallMcpTool(req, metadata=self._md))
        return CallToolResult(ok=resp.ok, result_json=resp.result_json, error=resp.error)

    @property
    def connections(self) -> _Connections:
        """The connector (external MCP server) admin namespace —
        ``kx.connections.add / list / test / remove / discover`` (the verb vocabulary
        of the ``kx connections`` CLI). The flat ``register_mcp_server`` etc. remain
        for back-compat. A connector is an external MCP tool server (see
        ``kx-extension-sdk``); chain one straight into a flow with
        ``kx.flow().with_mcp(...)``."""
        return _Connections(self)

    @property
    def memory(self) -> _Memory:
        """The durable agentic MEMORY namespace (RC5a) — ``kx.memory.store / list /
        recall / forget`` (the verb vocabulary of the ``kx memory`` CLI). Cross-run,
        per-principal memory the agent recalls in later runs. Chain seed facts into a
        flow with ``kx.flow().with_memory(...)``."""
        return _Memory(self)

    # --- D170 / MM-3: operator secret store (PutSecret / List / Delete) ---

    def put_secret(self, *, name: str, value: str) -> bool:
        """Store (create or overwrite) a named secret VALUE in the runtime's secret
        store (``PutSecret``). The value is held server-side (keychain / vault) and
        NEVER returned over the wire (D81); a connector ``credential_ref`` / a
        trigger ``auth_secret_ref`` later NAMES this row. Returns ``True`` iff the
        row was stored."""
        req = _g.PutSecretRequest(name=name, value=value)
        resp = self._call(lambda: self._stub.PutSecret(req, metadata=self._md))
        return resp.stored

    def list_secret_names(self, *, limit: int = 0, after_name: str = "") -> SecretNamesPage:
        """List the stored secret NAMES + audit timestamps (``ListSecretNames``),
        in ``(name)`` order. The secret VALUE is never on this wire (D81)."""
        req = _g.ListSecretNamesRequest(limit=limit, after_name=after_name)
        resp = self._call(lambda: self._stub.ListSecretNames(req, metadata=self._md))
        return SecretNamesPage(
            names=[SecretName.from_proto(s) for s in resp.names], has_more=resp.has_more
        )

    def delete_secret(self, *, name: str) -> bool:
        """Remove a named secret (``DeleteSecret``). Returns ``True`` iff a row was
        removed."""
        req = _g.DeleteSecretRequest(name=name)
        resp = self._call(lambda: self._stub.DeleteSecret(req, metadata=self._md))
        return resp.removed

    @property
    def secrets(self) -> _Secrets:
        """The operator secret-store admin namespace — ``kx.secrets.set / list /
        remove`` (D170 / MM-3). The flat ``put_secret`` etc. remain for
        back-compat. Secrets hold connector credentials / trigger auth VALUES
        server-side; the value never returns over the wire (D81) — a
        ``credential_ref`` / ``auth_secret_ref`` NAMES one of these rows."""
        return _Secrets(self)

    # --- D170 / D113: trigger admin (Register / List / Deregister / Submit / Test) ---

    def register_trigger(
        self,
        *,
        name: str,
        kind: str = "webhook",
        recipe_handle: str = "",
        auth: str = "none",
        auth_secret_ref: str = "",
        schedule_spec: str = "",
        enabled: bool = True,
    ) -> str:
        """Register a durable trigger that binds an inbound event to a published
        recipe (``RegisterTrigger``). ``kind`` is ``"webhook"`` | ``"cron"`` |
        ``"grpc"`` and ``auth`` is ``"none"`` | ``"hmac_sha256"`` | ``"bearer"``
        (mapped to the proto enums; an unknown string raises ``ValueError``).
        ``auth_secret_ref`` NAMES a stored secret (the HMAC key / bearer token
        resolves server-side, never in the client). Returns the server-derived
        ``trigger_id`` as hex."""
        req = _g.RegisterTriggerRequest(
            name=name,
            kind=trigger_kind_to_proto(kind),
            recipe_handle=recipe_handle,
            auth=trigger_auth_to_proto(auth),
            auth_secret_ref=auth_secret_ref,
            schedule_spec=schedule_spec,
            enabled=enabled,
        )
        resp = self._call(lambda: self._stub.RegisterTrigger(req, metadata=self._md))
        return hexids.encode(resp.trigger_id)

    def list_triggers(self, *, limit: int = 0, after_name: str = "") -> TriggersPage:
        """List the registered triggers (``ListTriggers``), in ``(name)`` order.
        The auth secret value is never on the wire — only ``auth_secret_present``
        (D81)."""
        req = _g.ListTriggersRequest(limit=limit, after_name=after_name)
        resp = self._call(lambda: self._stub.ListTriggers(req, metadata=self._md))
        return TriggersPage(
            triggers=[TriggerView.from_proto(t) for t in resp.triggers],
            has_more=resp.has_more,
        )

    def deregister_trigger(self, *, name: str) -> bool:
        """Remove a registered trigger (``DeregisterTrigger``). Returns ``True``
        iff a row was removed."""
        req = _g.DeregisterTriggerRequest(name=name)
        resp = self._call(lambda: self._stub.DeregisterTrigger(req, metadata=self._md))
        return resp.removed

    def submit_trigger(
        self, *, name: str, idempotency_key: str = "", payload_json: str = ""
    ) -> "tuple[str, bool]":
        """Fire a registered trigger by name (``SubmitTrigger``) — binds its recipe
        + submits a run with the (optional) ``payload_json``. A non-empty
        ``idempotency_key`` dedupes a retried fire (mapping to an existing run
        returns it with ``deduped=True``). Returns ``(instance_id_hex, deduped)``."""
        req = _g.SubmitTriggerRequest(
            name=name, idempotency_key=idempotency_key, payload_json=payload_json
        )
        resp = self._call(lambda: self._stub.SubmitTrigger(req, metadata=self._md))
        return hexids.encode(resp.instance_id), resp.deduped

    def test_trigger(self, *, name: str, payload_json: str = "") -> "tuple[bool, str]":
        """Dry-run a trigger's binding without submitting a run (``TestTrigger``) —
        validates the recipe handle + payload shape. Returns ``(ok, detail)``."""
        req = _g.TestTriggerRequest(name=name, payload_json=payload_json)
        resp = self._call(lambda: self._stub.TestTrigger(req, metadata=self._md))
        return resp.ok, resp.detail

    @property
    def triggers(self) -> _Triggers:
        """The trigger admin namespace — ``kx.triggers.add / list / test / fire /
        remove`` (D170 / D113). The flat ``register_trigger`` etc. remain for
        back-compat. A trigger binds an inbound webhook / cron / gRPC event to a
        published recipe."""
        return _Triggers(self)

    # --- D114 (HITL approval) + M11 (cost readout) -----------------------------

    def list_pending_approvals(self, *, limit: int = 0) -> PendingApprovalsPage:
        """List the world-mutating actions withheld awaiting operator approval
        (``ListPendingApprovals``). Display-only — no authority."""
        req = _g.ListPendingApprovalsRequest(limit=limit)
        resp = self._call(lambda: self._stub.ListPendingApprovals(req, metadata=self._md))
        return PendingApprovalsPage(
            approvals=[PendingApproval.from_proto(a) for a in resp.approvals]
        )

    def grant_approval(self, *, request_id: str, reason: str = "") -> bool:
        """Grant a pending approval (``GrantApproval``) — releases the staged action
        to fire exactly once. Returns ``True`` iff a decision was recorded."""
        req = _g.GrantApprovalRequest(request_id=hexids.as_bytes(request_id, 16), reason=reason)
        resp = self._call(lambda: self._stub.GrantApproval(req, metadata=self._md))
        return resp.granted

    def deny_approval(self, *, request_id: str, reason: str = "") -> bool:
        """Deny a pending approval (``DenyApproval``) — the gated chain dead-letters
        fail-closed. Returns ``True`` iff a decision was recorded."""
        req = _g.DenyApprovalRequest(request_id=hexids.as_bytes(request_id, 16), reason=reason)
        resp = self._call(lambda: self._stub.DenyApproval(req, metadata=self._md))
        return resp.denied

    def get_run_cost(self, *, instance_id: str) -> RunCost:
        """The run's DISPLAY-ONLY local spend estimate (``GetRunCost``) — priced
        turn/tool counters at the operator's micro-USD rates (not Cloud billing)."""
        req = _g.GetRunCostRequest(instance_id=hexids.as_bytes(instance_id, hexids.INSTANCE_LEN))
        resp = self._call(lambda: self._stub.GetRunCost(req, metadata=self._md))
        return RunCost.from_proto(resp)

    def score_run(self, *, instance_id: str) -> RunScore:
        """A live run's EXPECTATION-FREE quality summary (``ScoreRun``) — terminal
        reached, turns / tool-calls spent, budget burn, rejection count. The golden-suite
        gate (vs an expectation) runs offline via ``kx eval run``."""
        req = _g.ScoreRunRequest(instance_id=hexids.as_bytes(instance_id, hexids.INSTANCE_LEN))
        resp = self._call(lambda: self._stub.ScoreRun(req, metadata=self._md))
        return RunScore.from_proto(resp)

    @property
    def approvals(self) -> _Approvals:
        """The HITL approval namespace — ``kx.approvals.list_pending / grant / deny``
        (D114). Grant/deny release/reject a staged world-mutating action (SN-8)."""
        return _Approvals(self)

    @property
    def cost(self) -> _Cost:
        """The cost-spend guardrail namespace — ``kx.cost.get_run_cost`` (M11). A
        display-only local spend estimate, not Cloud billing."""
        return _Cost(self)

    @property
    def eval(self) -> _Eval:
        """The agentic-evaluation namespace — ``kx.eval.score_run`` (RC1/D172). An
        expectation-free per-run quality summary; the golden gate runs offline."""
        return _Eval(self)

    def submit_feedback(
        self,
        rating: str,
        message_id: str,
        *,
        instance_id: Optional[str] = None,
        mote_id: Optional[str] = None,
        content_ref: Optional[str] = None,
        comment: str = "",
        recipe_handle: str = "",
        model_id: str = "",
    ) -> str:
        """Record 👍/👎 feedback on an answer (PR-4.1) — a client-origin write into
        the gateway's rebuildable-to-empty ``feedback.db`` sidecar (advisory; never
        truth/identity/a digest input). ``rating`` is ``"up"``/``"down"``;
        ``message_id`` is the stable per-answer key (required). The caller principal
        + the returned ``feedback_id`` (hex) are SERVER-derived; re-rating the same
        answer OVERWRITES. An old gateway raises ``KxUnimplemented``."""
        req = _g.SubmitFeedbackRequest(
            rating=rating_to_proto(rating),
            message_id=message_id,
            comment=comment,
            recipe_handle=recipe_handle,
            model_id=model_id,
        )
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if mote_id is not None:
            req.mote_id = hexids.decode(mote_id)
        if content_ref is not None:
            req.content_ref = hexids.decode(content_ref)
        resp = self._call(lambda: self._stub.SubmitFeedback(req, metadata=self._md))
        return hexids.encode(resp.feedback_id)

    def list_feedback(
        self,
        *,
        instance_id: Optional[str] = None,
        limit: Optional[int] = None,
        before_rowid: Optional[int] = None,
    ) -> FeedbackPage:
        """Read back recorded feedback (newest-first, paginated; PR-4.1) from the
        gateway's ``feedback.db`` sidecar — audit/inspection only. ``instance_id``
        (hex) scopes to one run; ``before_rowid`` resumes below the last row's
        rowid. The server clamps ``limit`` to its max page. An old gateway (or one
        without the sidecar) raises ``KxUnimplemented``."""
        req = _g.ListFeedbackRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if limit is not None:
            req.limit = limit
        if before_rowid is not None:
            req.before_rowid = before_rowid
        resp = self._call(lambda: self._stub.ListFeedback(req, metadata=self._md))
        return FeedbackPage(
            rows=[FeedbackRow.from_proto(r) for r in resp.rows], has_more=resp.has_more
        )

    def list_recipes(self) -> List[str]:
        """List the invocable recipe handles the gateway provisions (the public
        recipe catalog)."""
        resp = self._call(
            lambda: self._stub.ListRecipes(_g.ListRecipesRequest(), metadata=self._md)
        )
        return [r.handle for r in resp.recipes]

    def list_recipe_summaries(self) -> List[RecipeInfo]:
        """The recipe catalog WITH each recipe's published workflow fingerprint
        (PR-2.1) — the join key for labeling durable ``RunSummary`` rows by
        recipe handle. ``recipe_fingerprint`` is ``""`` on a gateway predating
        the field (additive — degrade to unlabeled rows)."""
        resp = self._call(
            lambda: self._stub.ListRecipes(_g.ListRecipesRequest(), metadata=self._md)
        )
        return [RecipeInfo.from_proto(r) for r in resp.recipes]

    def get_recipe_form(self, handle: str) -> RecipeForm:
        """The free-param :class:`RecipeForm` for ``handle`` (render a form, then
        :meth:`invoke`). An unknown handle raises ``KxNotFound``."""
        resp = self._call(
            lambda: self._stub.GetRecipeForm(
                _g.GetRecipeFormRequest(handle=handle), metadata=self._md
            )
        )
        return RecipeForm.from_proto(resp)

    def search_recipes(
        self,
        intent: str,
        *,
        keywords: Optional[List[str]] = None,
        limit: Optional[int] = None,
    ) -> List[ScoredRecipe]:
        """ADVISORY recipe discovery (PR-4 Batch D) — rank the provisioned
        recipes against ``intent`` (+ optional ``keywords``), best-first, capped
        at ``limit``. SN-8: each ``score_bp`` is DISPLAY-ONLY (a hit SURFACES a
        recipe, never invokes one — :meth:`invoke` stays the authorization gate).
        An old gateway / a catalog with no ranker raises ``KxUnimplemented``."""
        req = _g.SearchRecipesRequest(intent=intent, keywords=keywords or [])
        if limit is not None:
            req.limit = limit
        resp = self._call(lambda: self._stub.SearchRecipes(req, metadata=self._md))
        return [ScoredRecipe.from_proto(s) for s in resp.ranked]

    def list_teams(self) -> List[TeamSummary]:
        """Enumerate the teams the gateway knows (UI-3 Systems viewer). VIEW-only."""
        resp = self._call(lambda: self._stub.ListTeams(_g.ListTeamsRequest(), metadata=self._md))
        return [TeamSummary.from_proto(t) for t in resp.teams]

    def list_team_members(self, team_id: str, *, asset_ref: Optional[str] = None) -> TeamMembers:
        """The members of ``team_id`` (+ each member's role/caps). When ``asset_ref``
        is given, each member's resolved warrant on that asset (membership ∩ grant,
        ⊆ the team) is populated. An unknown team raises ``KxNotFound``."""
        req = _g.ListTeamMembersRequest(team_id=team_id)
        if asset_ref is not None:
            req.asset_ref = asset_ref
        resp = self._call(lambda: self._stub.ListTeamMembers(req, metadata=self._md))
        return TeamMembers.from_proto(resp)

    def list_asset_grants(self, asset_ref: str) -> AssetGrants:
        """Every grant on ``asset_ref``, fold-classified root/delegated + active/
        revoked (the UI-3 sharing inspector). An unknown asset raises ``KxNotFound``."""
        resp = self._call(
            lambda: self._stub.ListAssetGrants(
                _g.ListAssetGrantsRequest(asset_ref=asset_ref), metadata=self._md
            )
        )
        return AssetGrants.from_proto(resp)

    def list_datasets(self) -> List[DatasetSummary]:
        """Enumerate the datasets (RAG corpora) the gateway holds (T3.7). A gateway
        built without the ``hnsw`` feature raises ``KxUnimplemented``."""
        resp = self._call(
            lambda: self._stub.ListDatasets(_g.ListDatasetsRequest(), metadata=self._md)
        )
        return [DatasetSummary.from_proto(d) for d in resp.datasets]

    def ingest_documents(
        self, dataset: str, documents: Sequence[Union[IngestDocument, bytes]]
    ) -> IngestResult:
        """Ingest ``documents`` into ``dataset`` (created on first ingest). Each doc
        carries ``content`` (always) + an OPTIONAL client-computed ``embedding`` (the
        FFI-free path); a vector-less doc needs a gateway with the ``inference``
        feature (else ``KxFailedPrecondition``). The server derives each doc's id from
        its content (SN-8); re-ingesting identical content is a no-op."""
        req = _g.IngestDocumentsRequest(dataset=dataset, documents=_to_documents(documents))
        resp = self._call(lambda: self._stub.IngestDocuments(req, metadata=self._md))
        return IngestResult.from_proto(resp)

    def query_dataset(
        self,
        dataset: str,
        *,
        text: Optional[str] = None,
        embedding: Optional[Sequence[float]] = None,
        k: int = 10,
        mode: RetrievalMode = RetrievalMode.DEFAULT,
        rerank: Optional[bool] = None,
    ) -> List[DatasetHit]:
        """Query ``dataset`` for the top-``k`` nearest chunks. Pass ``embedding``
        (the FFI-free client-vector path, takes precedence) or ``text`` (server-embed,
        needs the ``inference`` feature). ``mode`` (RC4a) selects dense vs hybrid
        (BM25 + dense); ``rerank`` (RC4c) overrides the operator's MMR diversity-rerank
        default per query (``None`` ⇒ the server default). Hits are ordered by the
        DISPLAY-ONLY score (SN-8). An unknown dataset raises ``KxNotFound``."""
        req = _g.QueryDatasetRequest(
            dataset=dataset,
            query_text=text or "",
            k=k,
            retrieval_mode=cast("_g.RetrievalMode", int(mode)),
        )
        if rerank is not None:
            req.rerank = rerank
        if embedding:
            req.query_embedding.extend(embedding)
        resp = self._call(lambda: self._stub.QueryDataset(req, metadata=self._md))
        return [DatasetHit.from_proto(h) for h in resp.hits]

    def fuzzy_discovery(
        self,
        dataset: str,
        *,
        text: Optional[str] = None,
        embedding: Optional[Sequence[float]] = None,
        k: int = 10,
        mode: RetrievalMode = RetrievalMode.DEFAULT,
    ) -> List[FuzzyHit]:
        """Slice-B advisory fuzzy-in / exact-out discovery over ``dataset`` (D151).
        Like :meth:`query_dataset`, but each :class:`FuzzyHit` carries ONLY the
        content-addressed ref + a DISPLAY-ONLY basis-point score (SN-8) — join back
        to bytes with an EXACT :meth:`get_content` on the ref. An old / ``hnsw``-less
        gateway raises ``KxUnimplemented``."""
        req = _g.FuzzyDiscoveryRequest(
            dataset=dataset,
            query_text=text or "",
            k=k,
            retrieval_mode=cast("_g.RetrievalMode", int(mode)),
        )
        if embedding:
            req.query_embedding.extend(embedding)
        resp = self._call(lambda: self._stub.FuzzyDiscovery(req, metadata=self._md))
        return [FuzzyHit.from_proto(h) for h in resp.hits]

    def list_tool_manifests(self) -> List[ToolManifest]:
        """Enumerate the registered tools' ADVISORY manifests (W1.A5 toolscout),
        in deterministic ``(tool_id, tool_version)`` order — ranking/display
        material ONLY (SN-8): a manifest can surface a tool, never grant one; the
        sole grant gate stays exact ``(name, version)`` equality in lowering + the
        broker. An old gateway raises ``KxUnimplemented``."""
        resp = self._call(
            lambda: self._stub.ListToolManifests(_g.ListToolManifestsRequest(), metadata=self._md)
        )
        return [ToolManifest.from_proto(m) for m in resp.manifests]

    def score_task_bundle(self, spec: BundleSpec) -> BundleScore:
        """Score a client-authored :class:`BundleSpec` against every registered
        manifest (W1.A5 toolscout) — ADVISORY/DISPLAY-ONLY (SN-8): integer
        basis-point ranks that never authorize. The verdict is a server-side
        DRY-RUN of the real lowering gate against the SERVER-built react warrant
        (no client warrant input); the lowered WorkflowDef is discarded — nothing
        submits, nothing journals. An old gateway raises ``KxUnimplemented``."""
        resp = self._call(lambda: self._stub.ScoreTaskBundle(spec.to_proto(), metadata=self._md))
        return BundleScore.from_proto(resp)

    # -- RC5a: durable agentic memory (also via the `kx.memory` sub-client) --
    def store_memory(
        self, content: "str | bytes", *, kind: MemoryKind = MemoryKind.SEMANTIC
    ) -> StoreResult:
        """Remember a fact for LATER runs to recall (RC5a). Content-addressed +
        idempotent (the same fact dedups to one memory). The CLI/SDK use the
        SERVER-EMBED path, so the gateway needs ``inference,hnsw`` + a model +
        ``KX_SERVE_MEMORY=1`` (else ``KxUnimplemented`` / ``KxFailedPrecondition``).
        Scoped to the caller's own principal."""
        body = content.encode("utf-8") if isinstance(content, str) else content
        req = _g.StoreMemoryRequest(
            content=body, kind=cast("_g.MemoryKind", int(kind)), namespace=""
        )
        resp = self._call(lambda: self._stub.StoreMemory(req, metadata=self._md))
        return StoreResult.from_proto(resp)

    def list_memories(
        self, *, instance_id: Optional[str] = None, limit: int = 0
    ) -> List[Memory]:
        """The episodic memory log, newest-first, optionally scoped to one run
        (``instance_id`` hex). An old / memory-less gateway raises ``KxUnimplemented``."""
        req = _g.ListMemoriesRequest(namespace="")
        if limit:
            req.limit = limit
        if instance_id:
            req.instance_id = hexids.decode(instance_id)
        resp = self._call(lambda: self._stub.ListMemories(req, metadata=self._md))
        return [Memory.from_proto(m) for m in resp.memories]

    def recall_memory(self, text: str, *, k: int = 5) -> List[MemoryHit]:
        """Recall the top-k memories most similar to ``text`` (RC5a). Each hit's
        ``score`` is DISPLAY-ONLY (SN-8). Scoped to the caller's own principal."""
        req = _g.RecallMemoryRequest(query_text=text, k=k, namespace="")
        resp = self._call(lambda: self._stub.RecallMemory(req, metadata=self._md))
        return [MemoryHit.from_proto(h) for h in resp.hits]

    def forget_memory(self, memory_id: str) -> bool:
        """Erase a memory by its content id (hex). Returns ``True`` if a row was
        removed. Scoped to the caller's own principal."""
        req = _g.ForgetMemoryRequest(memory_id=hexids.decode(memory_id), namespace="")
        resp = self._call(lambda: self._stub.ForgetMemory(req, metadata=self._md))
        return resp.forgotten

    # -- wait plumbing --
    def _await_terminal(
        self, instance: bytes, terminal: bytes, timeout: float, mode: str
    ) -> Result:
        if mode == "events":
            outcome = _wait.events_result(self._stub, self._md, instance, terminal, timeout)
        else:
            outcome = _wait.poll_result(self._stub, self._md, instance, terminal, timeout)
        return self._finish(outcome)

    def _await_any(self, instance: bytes, timeout: float) -> Result:
        """Wait for the FIRST committed Mote — the submit / run_chain path, which has no
        statically-known terminal (backs :meth:`Run.wait` for a workflow run)."""
        return self._finish(_wait.poll_any(self._stub, self._md, instance, timeout))

    @staticmethod
    def _finish(outcome: _wait.WaitOutcome) -> Result:
        result = Result.from_outcome(outcome)
        if outcome.state == _wait.WaitState.FAILED:
            from .errors import KxRunFailed

            raise KxRunFailed(
                "the run's terminal Mote failed",
                instance_id=result.instance_id,
                terminal_mote_id=result.terminal_mote_id or None,
            )
        if outcome.state == _wait.WaitState.RUNNING:
            from .errors import KxWaitTimeout

            raise KxWaitTimeout(
                "run still in progress (timed out); resume with get_projection / events",
                instance_id=result.instance_id,
                terminal_mote_id=result.terminal_mote_id or None,
            )
        return result


# --- async client ------------------------------------------------------------


class AsyncKxClient:
    """An asyncio client for a running ``kx serve`` gateway (mirrors :class:`KxClient`)."""

    def __init__(
        self,
        endpoint: str = DEFAULT_ENDPOINT,
        *,
        token: Optional[str] = None,
        token_file: Optional[str] = None,
        default_model: str = "",
        channel_options: Optional[list] = None,
    ) -> None:
        self.endpoint = endpoint
        self._token = _resolve_token(endpoint, token, token_file)
        # Batch A: see `KxClient.default_model`.
        self.default_model = default_model or os.environ.get(DEFAULT_MODEL_ENV, "")
        self._md = [("authorization", f"Bearer {self._token}")] if self._token else []
        if endpoint.startswith("https://"):
            self._channel = grpc.aio.secure_channel(
                _target(endpoint),
                grpc.ssl_channel_credentials(),
                options=_merged_options(channel_options),
            )
        else:
            self._channel = grpc.aio.insecure_channel(
                _target(endpoint), options=_merged_options(channel_options)
            )
        self._stub = _gg.KxGatewayStub(self._channel)

    async def close(self) -> None:
        await self._channel.close()

    async def __aenter__(self) -> "AsyncKxClient":
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.close()

    async def _acall(self, coro):
        try:
            return await coro
        except grpc.RpcError as e:
            raise from_rpc_error(e) from e

    async def invoke(
        self,
        handle: str,
        args: ArgsType,
        *,
        wait: bool = False,
        timeout: float = 120.0,
        wait_mode: str = "poll",
        out: Optional[str] = None,
        context: Optional[Sequence[str]] = None,
        context_refs: Optional[Sequence[str]] = None,
    ) -> Union[AsyncRun, Result]:
        resp = await self._acall(
            self._stub.Invoke(
                _g.InvokeRequest(
                    handle=handle,
                    args=_encode_args(args),
                    context_bundles=list(context or []),
                    context_refs=list(context_refs or []),
                ),
                metadata=self._md,
            )
        )
        run = AsyncRun(self, resp.instance_id, resp.terminal_mote_id, resp.recipe_fingerprint)
        if not wait:
            return run
        if _is_react_handle(handle):
            # F13: a react chain settles via ListReactTurns, not a terminal Mote.
            # PR-R1: scope the settle poll to THIS invocation's chain.
            outcome = await _wait.apoll_react_result(
                self._stub,
                self._md,
                resp.instance_id,
                resp.terminal_mote_id,
                timeout,
                resp.react_chain_salt,
            )
            result = _dataclasses.replace(
                KxClient._finish(outcome),
                react_chain_salt=hexids.encode(resp.react_chain_salt)
                if resp.react_chain_salt
                else "",
            )
        else:
            result = await self._await_terminal(
                resp.instance_id, resp.terminal_mote_id, timeout, wait_mode
            )
        if out is not None and result.payload is not None:
            with open(out, "wb") as fh:
                fh.write(result.payload)
        return result

    async def _resolve_image_ref(self, image: ImageInput) -> str:
        """Async mirror of :meth:`KxClient._resolve_image_ref` (PR-B2)."""
        if isinstance(image, (bytes, bytearray)):
            return (await self.put_content(bytes(image))).content_ref
        if isinstance(image, dict):
            if "ref" in image:
                return str(image["ref"])
            if "bytes" in image:
                return (
                    await self.put_content(
                        bytes(image["bytes"]), media_type=image.get("media_type", "")
                    )
                ).content_ref
        raise KxUsage("image must be bytes, {'ref': <hex>}, or {'bytes': ..., 'media_type': ...}")

    async def _bind_vision(self, prompt: str, image_ref: str) -> Tuple[str, dict]:
        """Async mirror of :meth:`KxClient._bind_vision` (PR-B2)."""
        try:
            form = await self.get_recipe_form(VISION_RECIPE_HANDLE)
        except Exception as e:
            raise KxUsage(
                "vision is not available on this serve (no image-capable model). Pull/serve a "
                "vision model (e.g. gemma3 via Ollama, or Gemma-4 + mmproj via llama.cpp)."
            ) from e
        by = {f.name: f for f in form.fields}
        if "image_ref" not in by:
            raise KxUsage("the kx/recipes/vision form does not declare an image_ref slot")
        args: dict = {"image_ref": image_ref}
        if "prompt" in by:
            args["prompt"] = prompt
        model = by.get("model")
        if model is not None:
            args["model"] = (
                self.default_model
                if (self.default_model and self.default_model in model.allowed)
                else model.allowed[0]
            )
        return VISION_RECIPE_HANDLE, args

    async def _bind_react_vision(self, args: dict, image_ref: str) -> Tuple[str, dict]:
        """Async mirror of :meth:`KxClient._bind_react_vision` (AGENTIC-VISION)."""
        try:
            form = await self.get_recipe_form(REACT_VISION_RECIPE_HANDLE)
        except Exception as e:
            raise KxUsage(
                "agentic vision is not available on this serve (no image-capable model). "
                "Serve a vision model (e.g. gemma3 via Ollama, or Gemma-4 + mmproj via llama.cpp)."
            ) from e
        if "image_ref" not in {f.name for f in form.fields}:
            raise KxUsage("the kx/recipes/react-vision form does not declare an image_ref slot")
        return REACT_VISION_RECIPE_HANDLE, {**args, "image_ref": image_ref}

    async def _bind_vision_rag(
        self, prompt: str, image_ref: str, dataset: str, k: int
    ) -> Tuple[str, dict]:
        """Async mirror of :meth:`KxClient._bind_vision_rag` (RC4b vision-RAG)."""
        try:
            form = await self.get_recipe_form(VISION_RAG_RECIPE_HANDLE)
        except Exception as e:  # recipe not provisioned (text-only / non-hnsw / old gateway)
            raise KxUsage(
                "vision-RAG is not available on this serve — it needs BOTH an image-capable "
                "model AND the dataset (hnsw) features. Drop 'dataset' for a plain vision "
                "answer, or serve a vision model with datasets enabled."
            ) from e
        by = {f.name: f for f in form.fields}
        if "image_ref" not in by:
            raise KxUsage("the kx/recipes/vision-rag form does not declare an image_ref slot")
        args: dict = {"image_ref": image_ref, "dataset": dataset, "k": k}
        if "prompt" in by:
            args["prompt"] = prompt
        model = by.get("model")
        if model is not None:
            args["model"] = (
                self.default_model
                if (self.default_model and self.default_model in model.allowed)
                else model.allowed[0]
            )
        return VISION_RAG_RECIPE_HANDLE, args

    async def chat(
        self,
        prompt: str,
        *,
        dataset: Optional[str] = None,
        k: int = 4,
        timeout: float = 120.0,
        image: Optional[ImageInput] = None,
    ) -> str:
        """Async mirror of :meth:`KxClient.chat` — ask the served model a single
        question (optionally AUTO-RAG-grounded against ``dataset``, or image-bearing
        via ``image`` → ``kx/recipes/vision``) and return the committed answer text.
        The server degrades honestly to a plain answer when the dataset is
        missing/empty (never fakes grounding); RC4b: ``dataset`` + ``image`` together
        binds ``kx/recipes/vision-rag`` (image answer grounded on retrieved text)."""
        if image is not None:
            image_ref = await self._resolve_image_ref(image)
            if dataset is not None:
                v_handle, v_args = await self._bind_vision_rag(prompt, image_ref, dataset, k)
            else:
                v_handle, v_args = await self._bind_vision(prompt, image_ref)
            v_result = await self.invoke(v_handle, v_args, wait=True, timeout=timeout)
            assert isinstance(v_result, Result)
            return v_result.text or ""
        if dataset is not None:
            handle = CHAT_RAG_RECIPE_HANDLE
            args: dict = {"prompt": prompt, "dataset": dataset, "k": k}
        else:
            handle = CHAT_RECIPE_HANDLE
            args = {"prompt": prompt}
        result = await self.invoke(handle, args, wait=True, timeout=timeout)
        assert isinstance(result, Result)  # narrow for mypy (never an AsyncRun here)
        return result.text or ""

    async def submit_run(
        self, request: "_g.SubmitRunRequest", *, wait: bool = False, timeout: float = 120.0
    ) -> Union["_g.RunHandle", Result]:
        handle = await self._acall(self._stub.SubmitRun(request, metadata=self._md))
        if not wait:
            return handle
        outcome = await _wait.apoll_any(self._stub, self._md, handle.instance_id, timeout)
        return KxClient._finish(outcome)

    async def submit_workflow(
        self, request: "_g.SubmitWorkflowRequest", *, wait: bool = False, timeout: float = 120.0
    ) -> Union[AsyncRun, Result]:
        _fill_default_model(request, self.default_model)
        handle = await self._acall(self._stub.SubmitWorkflow(request, metadata=self._md))
        if not wait:
            return AsyncRun(self, handle.instance_id, b"", handle.recipe_fingerprint)
        outcome = await _wait.apoll_any(self._stub, self._md, handle.instance_id, timeout)
        return KxClient._finish(outcome)

    async def run_chain(
        self, chain: "_chains.Chain", *, wait: bool = False, timeout: float = 120.0
    ) -> Union[AsyncRun, Result]:
        """As :meth:`KxClient.run_chain` — lower a :class:`~kortecx.chains.Chain` and
        run it over :meth:`submit_workflow` (V2a: returns an :class:`~kortecx.run.AsyncRun`
        handle when ``wait=False``). V2b local tools (if any) are registered + resolved
        first."""
        from .tools import aresolve_local_tools

        await aresolve_local_tools(self, chain)
        return await self.submit_workflow(chain.build(), wait=wait, timeout=timeout)

    async def get_projection(
        self, instance_id: IdType, *, at_seq: Optional[int] = None
    ) -> types.Projection:
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        req = _g.GetProjectionRequest(instance_id=inst)
        if at_seq is not None:
            req.at_seq = at_seq
        view = await self._acall(self._stub.GetProjection(req, metadata=self._md))
        return types.Projection.from_proto(view)

    async def get_content(self, ref: IdType, instance_id: Optional[IdType] = None) -> bytes:
        """As :meth:`KxClient.get_content` — ``None`` reads the uploads scope."""
        cref = hexids.as_bytes(ref, hexids.REF_LEN)
        inst = b"" if instance_id is None else hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        blob = await self._acall(
            self._stub.GetContent(
                _g.GetContentRequest(content_ref=cref, instance_id=inst), metadata=self._md
            )
        )
        return blob.payload

    async def put_content(
        self, payload: bytes, *, media_type: str = "", filename: str = ""
    ) -> PutResult:
        """As :meth:`KxClient.put_content` (Batch A client upload)."""
        resp = await self._acall(
            self._stub.PutContent(
                _g.PutContentRequest(payload=payload, media_type=media_type, filename=filename),
                metadata=self._md,
            )
        )
        return PutResult.from_proto(resp)

    async def get_content_batch(
        self,
        refs: Sequence[IdType],
        *,
        instance_id: Optional[IdType] = None,
        max_bytes_per_item: Optional[int] = None,
    ) -> List[ContentItem]:
        """As :meth:`KxClient.get_content_batch` (Batch A batch read)."""
        content_refs = [hexids.as_bytes(r, hexids.REF_LEN) for r in refs]
        inst = b"" if instance_id is None else hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        req = _g.GetContentBatchRequest(instance_id=inst, content_refs=content_refs)
        if max_bytes_per_item is not None:
            req.max_bytes_per_item = max_bytes_per_item
        resp = await self._acall(self._stub.GetContentBatch(req, metadata=self._md))
        return [ContentItem.from_proto(i) for i in resp.items]

    async def list_models(self) -> List[ModelSummary]:
        """As :meth:`KxClient.list_models` (Batch A model discovery + POC-3 live
        residency / chat handle)."""
        resp = await self._acall(self._stub.ListModels(_g.ListModelsRequest(), metadata=self._md))
        return [ModelSummary.from_proto(m) for m in resp.models]

    async def load_model(self, model_id: str) -> ModelLifecycleResult:
        """Async mirror of :meth:`KxClient.load_model` (POC-3 warm a model)."""
        resp = await self._acall(
            self._stub.LoadModel(_g.LoadModelRequest(model_id=model_id), metadata=self._md)
        )
        return ModelLifecycleResult.from_load(resp)

    async def offload_model(self, model_id: str) -> ModelLifecycleResult:
        """Async mirror of :meth:`KxClient.offload_model` (POC-3 evict a model)."""
        resp = await self._acall(
            self._stub.OffloadModel(_g.OffloadModelRequest(model_id=model_id), metadata=self._md)
        )
        return ModelLifecycleResult.from_offload(resp)

    async def get_server_info(self) -> ServerInfo:
        """Async mirror of :meth:`KxClient.get_server_info` (POC-1 Settings) — the
        connected gateway's effective config (display/settings-only, never a
        secret, SN-8). An old gateway raises ``KxUnimplemented``."""
        resp = await self._acall(
            self._stub.GetServerInfo(_g.GetServerInfoRequest(), metadata=self._md)
        )
        return ServerInfo.from_proto(resp)

    async def pull_model(
        self,
        *,
        ollama_tag: Optional[str] = None,
        url: Optional[str] = None,
        sha256: Optional[str] = None,
    ) -> str:
        """Async mirror of :meth:`KxClient.pull_model` (Model Control v2 — download +
        runtime-register a model). Returns the ``model_id`` to poll."""
        if (ollama_tag is None) == (url is None):
            raise ValueError("pull_model requires exactly one of ollama_tag or url")
        req = _g.PullModelRequest(sha256=sha256 or "")
        if ollama_tag is not None:
            req.ollama_tag = ollama_tag
        else:
            req.url = url or ""
        resp = await self._acall(self._stub.PullModel(req, metadata=self._md))
        if not resp.accepted:
            from .errors import KxFailedPrecondition

            raise KxFailedPrecondition(f"pull refused: {resp.detail}")
        return resp.model_id

    async def get_pull_status(self, model_id: str) -> PullStatus:
        """Async mirror of :meth:`KxClient.get_pull_status` (Model Control v2)."""
        resp = await self._acall(
            self._stub.GetPullStatus(_g.GetPullStatusRequest(model_id=model_id), metadata=self._md)
        )
        return PullStatus.from_proto(model_id, resp)

    async def set_active_model(self, model_id: str = "") -> str:
        """Async mirror of :meth:`KxClient.set_active_model` (Model Control v2)."""
        resp = await self._acall(
            self._stub.SetActiveModel(
                _g.SetActiveModelRequest(model_id=model_id), metadata=self._md
            )
        )
        return resp.active_model_id

    def stream_events(
        self, instance_id: IdType, *, since: int = 0, follow: bool = False
    ) -> AsyncIterator[types.Delta]:
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        return _events.astream_deltas(self._stub, self._md, inst, since, follow)

    def stream_model_tokens(
        self, instance_id: IdType, mote_id: IdType, *, since: int = 0
    ) -> AsyncIterator[types.TokenChunk]:
        """Async :meth:`KxClient.stream_model_tokens` (PR-4.2 ADVISORY token tail)."""
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        mote = hexids.as_bytes(mote_id, hexids.REF_LEN)
        return _events.astream_model_tokens(self._stub, self._md, inst, mote, since)

    def stream_all_events(
        self, *, since: int = 0, follow: bool = False
    ) -> AsyncIterator[types.GlobalDelta]:
        """Async :meth:`KxClient.stream_all_events` (the Batch C global tail)."""
        return _events.astream_all_deltas(self._stub, self._md, since, follow)

    async def list_signatures(self) -> List[types.SignatureSummary]:
        resp = await self._acall(
            self._stub.ListSignatures(_g.ListSignaturesRequest(), metadata=self._md)
        )
        return [types.SignatureSummary.from_proto(s) for s in resp.signatures]

    async def get_signature(self, signature_id: IdType) -> bytes:
        sid = hexids.as_bytes(signature_id, hexids.REF_LEN)
        resp = await self._acall(
            self._stub.GetSignature(_g.GetSignatureRequest(signature_id=sid), metadata=self._md)
        )
        return resp.manifest

    async def register_signature(self, manifest: bytes) -> str:
        resp = await self._acall(
            self._stub.RegisterSignature(
                _g.RegisterSignatureRequest(manifest=manifest), metadata=self._md
            )
        )
        return hexids.encode(resp.signature_id)

    async def list_runs(
        self, *, limit: Optional[int] = None, before_seq: Optional[int] = None
    ) -> RunPage:
        req = _g.ListRunsRequest()
        if limit is not None:
            req.limit = limit
        if before_seq is not None:
            req.before_seq = before_seq
        resp = await self._acall(self._stub.ListRuns(req, metadata=self._md))
        return RunPage(runs=[RunSummary.from_proto(r) for r in resp.runs], has_more=resp.has_more)

    async def get_run_inputs(self, instance_id: IdType) -> RunInputs:
        """The args a run was submitted with (PR-D ``GetRunInputs``) — the baseline
        for "Re-run with changes". A run with nothing captured raises
        ``KxNotFound``; an old gateway raises ``KxUnimplemented``."""
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        resp = await self._acall(
            self._stub.GetRunInputs(_g.GetRunInputsRequest(instance_id=inst), metadata=self._md)
        )
        return RunInputs.from_proto(resp)

    async def get_mote_detail(self, instance_id: IdType, mote_id: IdType) -> MoteDetail:
        """Async :meth:`KxClient.get_mote_detail`."""
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        mote = hexids.as_bytes(mote_id, hexids.REF_LEN)
        resp = await self._acall(
            self._stub.GetMoteDetail(
                _g.GetMoteDetailRequest(instance_id=inst, mote_id=mote), metadata=self._md
            )
        )
        return MoteDetail.from_proto(resp)

    async def list_react_turns(
        self,
        *,
        instance_id: Optional[str] = None,
        step_salt: Optional[str] = None,
        limit: Optional[int] = None,
    ) -> ReactTurnPage:
        """Async :meth:`KxClient.list_react_turns`."""
        req = _g.ListReactTurnsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if step_salt:
            req.step_salt = hexids.decode(step_salt)
        if limit is not None:
            req.limit = limit
        resp = await self._acall(self._stub.ListReactTurns(req, metadata=self._md))
        return ReactTurnPage(
            turns=[ReactTurn.from_proto(t) for t in resp.turns], has_more=resp.has_more
        )

    async def list_replan_rounds(self, *, limit: Optional[int] = None) -> ReplanRoundPage:
        """Async :meth:`KxClient.list_replan_rounds`."""
        req = _g.ListReplanRoundsRequest()
        if limit is not None:
            req.limit = limit
        resp = await self._acall(self._stub.ListReplanRounds(req, metadata=self._md))
        return ReplanRoundPage(
            rounds=[ReplanRound.from_proto(r) for r in resp.rounds], has_more=resp.has_more
        )

    async def list_rerank_turns(
        self, *, instance_id: Optional[str] = None, limit: Optional[int] = None
    ) -> ReRankTurnPage:
        """Async :meth:`KxClient.list_rerank_turns`."""
        req = _g.ListReRankTurnsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if limit is not None:
            req.limit = limit
        resp = await self._acall(self._stub.ListReRankTurns(req, metadata=self._md))
        return ReRankTurnPage(
            turns=[ReRankTurn.from_proto(t) for t in resp.turns], has_more=resp.has_more
        )

    async def list_capture_records(
        self, *, instance_id: Optional[str] = None, limit: Optional[int] = None
    ) -> CaptureRecordPage:
        """Async :meth:`KxClient.list_capture_records`."""
        req = _g.ListCaptureRecordsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if limit is not None:
            req.limit = limit
        resp = await self._acall(self._stub.ListCaptureRecords(req, metadata=self._md))
        return CaptureRecordPage(
            records=[CaptureRecord.from_proto(r) for r in resp.records], has_more=resp.has_more
        )

    async def list_mote_telemetry(
        self,
        *,
        instance_id: Optional[str] = None,
        mote_id: Optional[str] = None,
        limit: Optional[int] = None,
        before_seq: Optional[int] = None,
    ) -> TelemetryPage:
        """Async :meth:`KxClient.list_mote_telemetry`."""
        req = _g.ListMoteTelemetryRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if mote_id is not None:
            req.mote_id = hexids.decode(mote_id)
        if limit is not None:
            req.limit = limit
        if before_seq is not None:
            req.before_seq = before_seq
        resp = await self._acall(self._stub.ListMoteTelemetry(req, metadata=self._md))
        return TelemetryPage(
            rows=[MoteTelemetryRow.from_proto(r) for r in resp.rows], has_more=resp.has_more
        )

    async def list_telemetry_summary(
        self,
        *,
        instance_id: Optional[str] = None,
    ) -> TelemetrySummary:
        """Async :meth:`KxClient.list_telemetry_summary`."""
        req = _g.ListTelemetrySummaryRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        resp = await self._acall(self._stub.ListTelemetrySummary(req, metadata=self._md))
        return TelemetrySummary.from_proto(resp)

    async def submit_feedback(
        self,
        rating: str,
        message_id: str,
        *,
        instance_id: Optional[str] = None,
        mote_id: Optional[str] = None,
        content_ref: Optional[str] = None,
        comment: str = "",
        recipe_handle: str = "",
        model_id: str = "",
    ) -> str:
        """Async :meth:`KxClient.submit_feedback`."""
        req = _g.SubmitFeedbackRequest(
            rating=rating_to_proto(rating),
            message_id=message_id,
            comment=comment,
            recipe_handle=recipe_handle,
            model_id=model_id,
        )
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if mote_id is not None:
            req.mote_id = hexids.decode(mote_id)
        if content_ref is not None:
            req.content_ref = hexids.decode(content_ref)
        resp = await self._acall(self._stub.SubmitFeedback(req, metadata=self._md))
        return hexids.encode(resp.feedback_id)

    async def list_feedback(
        self,
        *,
        instance_id: Optional[str] = None,
        limit: Optional[int] = None,
        before_rowid: Optional[int] = None,
    ) -> FeedbackPage:
        """Async :meth:`KxClient.list_feedback`."""
        req = _g.ListFeedbackRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
        if limit is not None:
            req.limit = limit
        if before_rowid is not None:
            req.before_rowid = before_rowid
        resp = await self._acall(self._stub.ListFeedback(req, metadata=self._md))
        return FeedbackPage(
            rows=[FeedbackRow.from_proto(r) for r in resp.rows], has_more=resp.has_more
        )

    async def list_recipes(self) -> List[str]:
        resp = await self._acall(self._stub.ListRecipes(_g.ListRecipesRequest(), metadata=self._md))
        return [r.handle for r in resp.recipes]

    async def list_recipe_summaries(self) -> List[RecipeInfo]:
        """Async :meth:`KxClient.list_recipe_summaries`."""
        resp = await self._acall(self._stub.ListRecipes(_g.ListRecipesRequest(), metadata=self._md))
        return [RecipeInfo.from_proto(r) for r in resp.recipes]

    async def get_recipe_form(self, handle: str) -> RecipeForm:
        resp = await self._acall(
            self._stub.GetRecipeForm(_g.GetRecipeFormRequest(handle=handle), metadata=self._md)
        )
        return RecipeForm.from_proto(resp)

    async def search_recipes(
        self,
        intent: str,
        *,
        keywords: Optional[List[str]] = None,
        limit: Optional[int] = None,
    ) -> List[ScoredRecipe]:
        """Async :meth:`KxClient.search_recipes`."""
        req = _g.SearchRecipesRequest(intent=intent, keywords=keywords or [])
        if limit is not None:
            req.limit = limit
        resp = await self._acall(self._stub.SearchRecipes(req, metadata=self._md))
        return [ScoredRecipe.from_proto(s) for s in resp.ranked]

    async def list_teams(self) -> List[TeamSummary]:
        resp = await self._acall(self._stub.ListTeams(_g.ListTeamsRequest(), metadata=self._md))
        return [TeamSummary.from_proto(t) for t in resp.teams]

    async def list_team_members(
        self, team_id: str, *, asset_ref: Optional[str] = None
    ) -> TeamMembers:
        req = _g.ListTeamMembersRequest(team_id=team_id)
        if asset_ref is not None:
            req.asset_ref = asset_ref
        resp = await self._acall(self._stub.ListTeamMembers(req, metadata=self._md))
        return TeamMembers.from_proto(resp)

    async def list_asset_grants(self, asset_ref: str) -> AssetGrants:
        resp = await self._acall(
            self._stub.ListAssetGrants(
                _g.ListAssetGrantsRequest(asset_ref=asset_ref), metadata=self._md
            )
        )
        return AssetGrants.from_proto(resp)

    async def list_datasets(self) -> List[DatasetSummary]:
        resp = await self._acall(
            self._stub.ListDatasets(_g.ListDatasetsRequest(), metadata=self._md)
        )
        return [DatasetSummary.from_proto(d) for d in resp.datasets]

    async def ingest_documents(
        self, dataset: str, documents: Sequence[Union[IngestDocument, bytes]]
    ) -> IngestResult:
        req = _g.IngestDocumentsRequest(dataset=dataset, documents=_to_documents(documents))
        resp = await self._acall(self._stub.IngestDocuments(req, metadata=self._md))
        return IngestResult.from_proto(resp)

    async def query_dataset(
        self,
        dataset: str,
        *,
        text: Optional[str] = None,
        embedding: Optional[Sequence[float]] = None,
        k: int = 10,
        mode: RetrievalMode = RetrievalMode.DEFAULT,
        rerank: Optional[bool] = None,
    ) -> List[DatasetHit]:
        """Async twin of :meth:`KxClient.query_dataset` (RC4a ``mode`` selects dense
        vs hybrid; RC4c ``rerank`` overrides the MMR default per query)."""
        req = _g.QueryDatasetRequest(
            dataset=dataset,
            query_text=text or "",
            k=k,
            retrieval_mode=cast("_g.RetrievalMode", int(mode)),
        )
        if rerank is not None:
            req.rerank = rerank
        if embedding:
            req.query_embedding.extend(embedding)
        resp = await self._acall(self._stub.QueryDataset(req, metadata=self._md))
        return [DatasetHit.from_proto(h) for h in resp.hits]

    # -- RC5a: durable agentic memory (async twins) --
    async def store_memory(
        self, content: "str | bytes", *, kind: MemoryKind = MemoryKind.SEMANTIC
    ) -> StoreResult:
        """Async twin of :meth:`KxClient.store_memory`."""
        body = content.encode("utf-8") if isinstance(content, str) else content
        req = _g.StoreMemoryRequest(
            content=body, kind=cast("_g.MemoryKind", int(kind)), namespace=""
        )
        resp = await self._acall(self._stub.StoreMemory(req, metadata=self._md))
        return StoreResult.from_proto(resp)

    async def list_memories(
        self, *, instance_id: Optional[str] = None, limit: int = 0
    ) -> List[Memory]:
        """Async twin of :meth:`KxClient.list_memories`."""
        req = _g.ListMemoriesRequest(namespace="")
        if limit:
            req.limit = limit
        if instance_id:
            req.instance_id = hexids.decode(instance_id)
        resp = await self._acall(self._stub.ListMemories(req, metadata=self._md))
        return [Memory.from_proto(m) for m in resp.memories]

    async def recall_memory(self, text: str, *, k: int = 5) -> List[MemoryHit]:
        """Async twin of :meth:`KxClient.recall_memory`."""
        req = _g.RecallMemoryRequest(query_text=text, k=k, namespace="")
        resp = await self._acall(self._stub.RecallMemory(req, metadata=self._md))
        return [MemoryHit.from_proto(h) for h in resp.hits]

    async def forget_memory(self, memory_id: str) -> bool:
        """Async twin of :meth:`KxClient.forget_memory`."""
        req = _g.ForgetMemoryRequest(memory_id=hexids.decode(memory_id), namespace="")
        resp = await self._acall(self._stub.ForgetMemory(req, metadata=self._md))
        return resp.forgotten

    async def fuzzy_discovery(
        self,
        dataset: str,
        *,
        text: Optional[str] = None,
        embedding: Optional[Sequence[float]] = None,
        k: int = 10,
        mode: RetrievalMode = RetrievalMode.DEFAULT,
    ) -> List[FuzzyHit]:
        """Slice-B advisory fuzzy-in / exact-out discovery (D151) — async twin of
        :meth:`KxClient.fuzzy_discovery`. Returns refs + DISPLAY-ONLY basis-point
        scores (SN-8); join back to bytes with an EXACT ``get_content``."""
        req = _g.FuzzyDiscoveryRequest(
            dataset=dataset,
            query_text=text or "",
            k=k,
            retrieval_mode=cast("_g.RetrievalMode", int(mode)),
        )
        if embedding:
            req.query_embedding.extend(embedding)
        resp = await self._acall(self._stub.FuzzyDiscovery(req, metadata=self._md))
        return [FuzzyHit.from_proto(h) for h in resp.hits]

    async def list_tool_manifests(self) -> List[ToolManifest]:
        """Async :meth:`KxClient.list_tool_manifests`."""
        resp = await self._acall(
            self._stub.ListToolManifests(_g.ListToolManifestsRequest(), metadata=self._md)
        )
        return [ToolManifest.from_proto(m) for m in resp.manifests]

    async def score_task_bundle(self, spec: BundleSpec) -> BundleScore:
        """Async :meth:`KxClient.score_task_bundle`."""
        resp = await self._acall(self._stub.ScoreTaskBundle(spec.to_proto(), metadata=self._md))
        return BundleScore.from_proto(resp)

    async def _await_terminal(
        self, instance: bytes, terminal: bytes, timeout: float, mode: str
    ) -> Result:
        if mode == "events":
            outcome = await _wait.aevents_result(self._stub, self._md, instance, terminal, timeout)
        else:
            outcome = await _wait.apoll_result(self._stub, self._md, instance, terminal, timeout)
        return KxClient._finish(outcome)

    async def _await_any(self, instance: bytes, timeout: float) -> Result:
        """Wait for the FIRST committed Mote (the submit / run_chain path — no static
        terminal; backs :meth:`AsyncRun.wait` for a workflow run)."""
        return KxClient._finish(await _wait.apoll_any(self._stub, self._md, instance, timeout))
