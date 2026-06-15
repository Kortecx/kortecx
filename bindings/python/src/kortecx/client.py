"""The kortecx gateway clients — :class:`KxClient` (sync) and :class:`AsyncKxClient`.

Thin, typed wrappers over the generated ``KxGatewayStub``: credential resolution,
channel lifecycle, and the eight RPCs, plus the high-level ``invoke(..., wait=True)``
"runtime as a function". Identity is server-derived (SN-8): the client sends a
*credential* (a bearer token), never a claimed identity, and never computes an id.
"""

from __future__ import annotations

import json
import os
import warnings
from typing import TYPE_CHECKING, AsyncIterator, Iterator, List, Optional, Sequence, Union

import grpc

if TYPE_CHECKING:
    from . import chains as _chains

from . import events as _events
from . import hexids, types
from . import wait as _wait  # aliased: `wait` is also a public kwarg name
from .capture import CaptureRecord, CaptureRecordPage
from .content import ContentItem, PutResult
from .datasets import (
    DatasetHit,
    DatasetSummary,
    IngestDocument,
    IngestResult,
    _to_documents,
)
from .errors import KxUsage, from_rpc_error
from .feedback import FeedbackPage, FeedbackRow, rating_to_proto
from .grants import AssetGrants
from .models import ModelSummary
from .motes import MoteDetail
from .react import ReactTurn, ReactTurnPage
from .recipes import RecipeForm, RecipeInfo, ScoredRecipe
from .replan import ReplanRound, ReplanRoundPage
from .run import AsyncRun, Result, Run
from .runs import RunInputs, RunPage, RunSummary
from .teams import TeamMembers, TeamSummary
from .telemetry import MoteTelemetryRow, TelemetryPage
from .toolscout import BundleScore, BundleSpec, ToolManifest
from .v1 import gateway_pb2 as _g
from .v1 import gateway_pb2_grpc as _gg

#: The conventional gateway endpoint (matches ``kx serve`` / the CLI default).
DEFAULT_ENDPOINT = "http://127.0.0.1:50151"

#: The canonical ReAct recipe handle. A react run has NO statically-known terminal
#: Mote (the gateway hands back a run-salted turn-0 id that never commits, and the
#: settled Answer turn isn't known until the model emits it), so ``invoke(wait=True)``
#: on this handle waits on chain SETTLEMENT via ``ListReactTurns`` instead of a
#: single terminal Mote (campaign finding F13).
REACT_RECIPE_HANDLE = "kx/recipes/react"

ArgsType = Union[dict, bytes, bytearray, str]
IdType = Union[str, bytes]

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
    """True for the ReAct recipe — its ``invoke(wait=True)`` settles via the chain
    (``ListReactTurns``), not a terminal Mote (F13)."""
    return handle == REACT_RECIPE_HANDLE


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


class KxClient:
    """A synchronous client for a running ``kx serve`` gateway."""

    def __init__(
        self,
        endpoint: str = DEFAULT_ENDPOINT,
        *,
        token: Optional[str] = None,
        token_file: Optional[str] = None,
        channel_options: Optional[list] = None,
    ) -> None:
        self.endpoint = endpoint
        self._token = _resolve_token(endpoint, token, token_file)
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
    ) -> Union[Run, Result]:
        """Bind a published recipe to ``args`` and run it.

        With ``wait=True`` blocks for the committed :class:`Result` (raising
        :class:`KxRunFailed` / :class:`KxWaitTimeout` on a failed / timed-out run);
        otherwise returns a :class:`Run` handle. ``wait_mode="events"`` uses the
        low-latency live subscription instead of polling.
        """
        resp = self._call(
            lambda: self._stub.Invoke(
                _g.InvokeRequest(handle=handle, args=_encode_args(args)), metadata=self._md
            )
        )
        run = Run(self, resp.instance_id, resp.terminal_mote_id, resp.recipe_fingerprint)
        if not wait:
            return run
        if _is_react_handle(handle):
            # F13: a react chain settles via ListReactTurns, not a terminal Mote.
            outcome = _wait.poll_react_result(
                self._stub, self._md, resp.instance_id, resp.terminal_mote_id, timeout
            )
            result = self._finish(outcome)
        else:
            result = self._await_terminal(
                resp.instance_id, resp.terminal_mote_id, timeout, wait_mode
            )
        if out is not None and result.payload is not None:
            with open(out, "wb") as fh:
                fh.write(result.payload)
        return result

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
    ) -> Union["_g.RunHandle", Result]:
        """Author a Tier-1 DAG (a :class:`BlueprintBuilder` ``build()``) and run it.
        The server COMPILES the DAG, derives all identity, and builds every warrant
        from the party's grants (SN-8) — the client sends only the topology + params.
        Returns the ``RunHandle``, or — with ``wait=True`` — the first committed
        :class:`Result`. An old gateway without the seam raises ``KxUnimplemented``."""
        handle = self._call(lambda: self._stub.SubmitWorkflow(request, metadata=self._md))
        if not wait:
            return handle
        outcome = _wait.poll_any(self._stub, self._md, handle.instance_id, timeout)
        return self._finish(outcome)

    def run_chain(
        self, chain: "_chains.Chain", *, wait: bool = False, timeout: float = 120.0
    ) -> Union["_g.RunHandle", Result]:
        """Lower a :class:`~kortecx.chains.Chain` (operator sugar or the string DSL)
        and run it. A thin convenience over :meth:`submit_workflow` — the server
        still COMPILES the DAG + builds every warrant from the party's grants
        (SN-8). Returns the ``RunHandle``, or — with ``wait=True`` — the first
        committed :class:`Result`."""
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

    def list_models(self) -> List[ModelSummary]:
        """Discover the models the connected gateway serves (Batch A). Display
        only (SN-8): selection stays a recipe ENUM free-param validated
        server-side. An FFI-free gateway returns an EMPTY list; an old gateway
        raises ``KxUnimplemented``."""
        resp = self._call(lambda: self._stub.ListModels(_g.ListModelsRequest(), metadata=self._md))
        return [ModelSummary.from_proto(m) for m in resp.models]

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
        self, *, instance_id: Optional[str] = None, limit: Optional[int] = None
    ) -> ReactTurnPage:
        """Enumerate a live ReAct chain's durable turn facts (newest-first,
        paginated) — the queryable Reason→Act→Observe history. ``instance_id``
        (hex) scopes to one run; absent enumerates every chain. The server
        clamps ``limit`` to its max page."""
        req = _g.ListReactTurnsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
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
    ) -> List[DatasetHit]:
        """Query ``dataset`` for the top-``k`` nearest documents. Pass ``embedding``
        (the FFI-free client-vector path, takes precedence) or ``text`` (server-embed,
        needs the ``inference`` feature). Hits are ordered by the DISPLAY-ONLY score
        (SN-8). An unknown dataset raises ``KxNotFound``."""
        req = _g.QueryDatasetRequest(dataset=dataset, query_text=text or "", k=k)
        if embedding:
            req.query_embedding.extend(embedding)
        resp = self._call(lambda: self._stub.QueryDataset(req, metadata=self._md))
        return [DatasetHit.from_proto(h) for h in resp.hits]

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

    # -- wait plumbing --
    def _await_terminal(
        self, instance: bytes, terminal: bytes, timeout: float, mode: str
    ) -> Result:
        if mode == "events":
            outcome = _wait.events_result(self._stub, self._md, instance, terminal, timeout)
        else:
            outcome = _wait.poll_result(self._stub, self._md, instance, terminal, timeout)
        return self._finish(outcome)

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
        channel_options: Optional[list] = None,
    ) -> None:
        self.endpoint = endpoint
        self._token = _resolve_token(endpoint, token, token_file)
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
    ) -> Union[AsyncRun, Result]:
        resp = await self._acall(
            self._stub.Invoke(
                _g.InvokeRequest(handle=handle, args=_encode_args(args)), metadata=self._md
            )
        )
        run = AsyncRun(self, resp.instance_id, resp.terminal_mote_id, resp.recipe_fingerprint)
        if not wait:
            return run
        if _is_react_handle(handle):
            # F13: a react chain settles via ListReactTurns, not a terminal Mote.
            outcome = await _wait.apoll_react_result(
                self._stub, self._md, resp.instance_id, resp.terminal_mote_id, timeout
            )
            result = KxClient._finish(outcome)
        else:
            result = await self._await_terminal(
                resp.instance_id, resp.terminal_mote_id, timeout, wait_mode
            )
        if out is not None and result.payload is not None:
            with open(out, "wb") as fh:
                fh.write(result.payload)
        return result

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
    ) -> Union["_g.RunHandle", Result]:
        handle = await self._acall(self._stub.SubmitWorkflow(request, metadata=self._md))
        if not wait:
            return handle
        outcome = await _wait.apoll_any(self._stub, self._md, handle.instance_id, timeout)
        return KxClient._finish(outcome)

    async def run_chain(
        self, chain: "_chains.Chain", *, wait: bool = False, timeout: float = 120.0
    ) -> Union["_g.RunHandle", Result]:
        """As :meth:`KxClient.run_chain` — lower a :class:`~kortecx.chains.Chain` and
        run it over :meth:`submit_workflow`."""
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
        """As :meth:`KxClient.list_models` (Batch A model discovery)."""
        resp = await self._acall(self._stub.ListModels(_g.ListModelsRequest(), metadata=self._md))
        return [ModelSummary.from_proto(m) for m in resp.models]

    def stream_events(
        self, instance_id: IdType, *, since: int = 0, follow: bool = False
    ) -> AsyncIterator[types.Delta]:
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        return _events.astream_deltas(self._stub, self._md, inst, since, follow)

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
        self, *, instance_id: Optional[str] = None, limit: Optional[int] = None
    ) -> ReactTurnPage:
        """Async :meth:`KxClient.list_react_turns`."""
        req = _g.ListReactTurnsRequest()
        if instance_id is not None:
            req.instance_id = hexids.decode(instance_id)
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
    ) -> List[DatasetHit]:
        req = _g.QueryDatasetRequest(dataset=dataset, query_text=text or "", k=k)
        if embedding:
            req.query_embedding.extend(embedding)
        resp = await self._acall(self._stub.QueryDataset(req, metadata=self._md))
        return [DatasetHit.from_proto(h) for h in resp.hits]

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
