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
from typing import AsyncIterator, Iterator, List, Optional, Sequence, Union

import grpc

from . import events as _events
from . import hexids, types
from . import wait as _wait  # aliased: `wait` is also a public kwarg name
from .capture import CaptureRecord, CaptureRecordPage
from .datasets import (
    DatasetHit,
    DatasetSummary,
    IngestDocument,
    IngestResult,
    _to_documents,
)
from .errors import KxUsage, from_rpc_error
from .grants import AssetGrants
from .react import ReactTurn, ReactTurnPage
from .recipes import RecipeForm
from .replan import ReplanRound, ReplanRoundPage
from .run import AsyncRun, Result, Run
from .runs import RunPage, RunSummary
from .teams import TeamMembers, TeamSummary
from .v1 import gateway_pb2 as _g
from .v1 import gateway_pb2_grpc as _gg

#: The conventional gateway endpoint (matches ``kx serve`` / the CLI default).
DEFAULT_ENDPOINT = "http://127.0.0.1:50151"

ArgsType = Union[dict, bytes, bytearray, str]
IdType = Union[str, bytes]


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
                _target(endpoint), grpc.ssl_channel_credentials(), options=channel_options
            )
        else:
            self._channel = grpc.insecure_channel(_target(endpoint), options=channel_options)
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
        result = self._await_terminal(resp.instance_id, resp.terminal_mote_id, timeout, wait_mode)
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

    def get_projection(
        self, instance_id: IdType, *, at_seq: Optional[int] = None
    ) -> types.Projection:
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        req = _g.GetProjectionRequest(instance_id=inst)
        if at_seq is not None:
            req.at_seq = at_seq
        view = self._call(lambda: self._stub.GetProjection(req, metadata=self._md))
        return types.Projection.from_proto(view)

    def get_content(self, ref: IdType, instance_id: IdType) -> bytes:
        cref = hexids.as_bytes(ref, hexids.REF_LEN)
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        blob = self._call(
            lambda: self._stub.GetContent(
                _g.GetContentRequest(content_ref=cref, instance_id=inst), metadata=self._md
            )
        )
        return blob.payload

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

    def list_recipes(self) -> List[str]:
        """List the invocable recipe handles the gateway provisions (the public
        recipe catalog)."""
        resp = self._call(
            lambda: self._stub.ListRecipes(_g.ListRecipesRequest(), metadata=self._md)
        )
        return [r.handle for r in resp.recipes]

    def get_recipe_form(self, handle: str) -> RecipeForm:
        """The free-param :class:`RecipeForm` for ``handle`` (render a form, then
        :meth:`invoke`). An unknown handle raises ``KxNotFound``."""
        resp = self._call(
            lambda: self._stub.GetRecipeForm(
                _g.GetRecipeFormRequest(handle=handle), metadata=self._md
            )
        )
        return RecipeForm.from_proto(resp)

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
                _target(endpoint), grpc.ssl_channel_credentials(), options=channel_options
            )
        else:
            self._channel = grpc.aio.insecure_channel(_target(endpoint), options=channel_options)
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

    async def get_projection(
        self, instance_id: IdType, *, at_seq: Optional[int] = None
    ) -> types.Projection:
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        req = _g.GetProjectionRequest(instance_id=inst)
        if at_seq is not None:
            req.at_seq = at_seq
        view = await self._acall(self._stub.GetProjection(req, metadata=self._md))
        return types.Projection.from_proto(view)

    async def get_content(self, ref: IdType, instance_id: IdType) -> bytes:
        cref = hexids.as_bytes(ref, hexids.REF_LEN)
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        blob = await self._acall(
            self._stub.GetContent(
                _g.GetContentRequest(content_ref=cref, instance_id=inst), metadata=self._md
            )
        )
        return blob.payload

    def stream_events(
        self, instance_id: IdType, *, since: int = 0, follow: bool = False
    ) -> AsyncIterator[types.Delta]:
        inst = hexids.as_bytes(instance_id, hexids.INSTANCE_LEN)
        return _events.astream_deltas(self._stub, self._md, inst, since, follow)

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

    async def list_recipes(self) -> List[str]:
        resp = await self._acall(self._stub.ListRecipes(_g.ListRecipesRequest(), metadata=self._md))
        return [r.handle for r in resp.recipes]

    async def get_recipe_form(self, handle: str) -> RecipeForm:
        resp = await self._acall(
            self._stub.GetRecipeForm(_g.GetRecipeFormRequest(handle=handle), metadata=self._md)
        )
        return RecipeForm.from_proto(resp)

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

    async def _await_terminal(
        self, instance: bytes, terminal: bytes, timeout: float, mode: str
    ) -> Result:
        if mode == "events":
            outcome = await _wait.aevents_result(self._stub, self._md, instance, terminal, timeout)
        else:
            outcome = await _wait.apoll_result(self._stub, self._md, instance, terminal, timeout)
        return KxClient._finish(outcome)
