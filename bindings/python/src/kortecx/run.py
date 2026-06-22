"""User-facing run handles and results.

:class:`Result` is the one-object answer of a ``wait`` (mirrors the CLI
``render_wait`` shape, so ``Result.to_dict()`` is byte-comparable to
``kx … --wait --json``). :class:`Run` / :class:`AsyncRun` are ergonomic handles
over a started run — ``.wait()``, ``.projection()``, ``.content()``, ``.events()``.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Iterator, Optional

from . import hexids
from .wait import WaitOutcome, WaitState

if TYPE_CHECKING:  # avoid an import cycle at runtime
    from .client import AsyncKxClient, KxClient
    from .types import Delta, Projection, TokenChunk


@dataclass(frozen=True)
class Result:
    """The terminal outcome of a ``wait`` — server-derived ids + the result."""

    instance_id: str  # hex (16B)
    terminal_mote_id: str  # hex (32B); "" on the submit-failure/timeout path
    state: str  # COMMITTED / FAILED / RUNNING
    result_ref: Optional[str]  # hex (32B) when committed
    payload: Optional[bytes]
    react_chain_salt: str = ""  # PR-R1: the react chain key (hex 32B); "" for a non-react run

    @classmethod
    def from_outcome(cls, o: WaitOutcome) -> "Result":
        return cls(
            instance_id=hexids.encode(o.instance_id),
            terminal_mote_id=hexids.encode(o.terminal_mote_id),
            state=o.state.value,
            result_ref=hexids.encode(o.result_ref) if o.result_ref is not None else None,
            payload=o.payload,
        )

    @property
    def ok(self) -> bool:
        """True iff the run committed."""
        return self.state == WaitState.COMMITTED.value

    @property
    def timed_out(self) -> bool:
        return self.state == WaitState.RUNNING.value

    @property
    def bytes(self) -> Optional[bytes]:
        """The committed result bytes (``None`` if not committed / no result)."""
        return self.payload

    @property
    def text(self) -> Optional[str]:
        """The committed result decoded as UTF-8 (``None`` if not text / absent)."""
        if self.payload is None:
            return None
        try:
            return self.payload.decode("utf-8")
        except UnicodeDecodeError:
            return None

    @property
    def verdict(self) -> Optional[str]:
        """T-AGENT2: if this run's terminal is an LLM-judge (``kx/recipes/judge``),
        the decoded ``"valid"`` / ``"invalid: <reason>"`` summary; ``None`` otherwise.
        Display-only (SN-8)."""
        from .critic import decode_critic_verdict

        if self.payload is None:
            return None
        return decode_critic_verdict(self.payload)

    def to_dict(self, include_payload: bool = True) -> dict:
        """The CLI ``--wait --json`` shape (parity with ``render_wait``)."""
        out: dict = {
            "instance_id": self.instance_id,
            "terminal_mote_id": self.terminal_mote_id,
            "state": self.state,
        }
        if self.result_ref is not None:
            out["result_ref"] = self.result_ref
        if self.timed_out:
            out["timed_out"] = True
        if self.payload is not None:
            out["result_len"] = len(self.payload)
            verdict = self.verdict
            if verdict is not None:
                out["verdict"] = verdict
            if include_payload:
                text = self.text
                if text is not None:
                    out["result_utf8"] = text
                out["result_hex"] = hexids.encode(self.payload)
        return out

    def json(self, include_payload: bool = True) -> dict:
        """The JSON-able dict shape — an alias of :meth:`to_dict` (mirrors the TS
        ``Result.json()``, so the two SDKs read the same)."""
        return self.to_dict(include_payload=include_payload)


class _RunBase:
    """Common id surface for a started run (server-derived; never client-computed)."""

    def __init__(self, instance_id: bytes, terminal_mote_id: bytes, recipe_fingerprint: bytes):
        self._instance = instance_id
        self._terminal = terminal_mote_id
        self._fingerprint = recipe_fingerprint

    @property
    def instance_id(self) -> str:
        """The run instance id (hex, 16B)."""
        return hexids.encode(self._instance)

    @property
    def terminal_mote_id(self) -> str:
        """The sink Mote whose committed result is this invocation's output (hex)."""
        return hexids.encode(self._terminal)

    @property
    def recipe_fingerprint(self) -> str:
        return hexids.encode(self._fingerprint)

    @property
    def instance_id_bytes(self) -> bytes:
        return self._instance

    @property
    def terminal_mote_id_bytes(self) -> bytes:
        return self._terminal


class Run(_RunBase):
    """A started run on a sync :class:`~kortecx.client.KxClient`."""

    def __init__(self, client: "KxClient", instance_id, terminal_mote_id, recipe_fingerprint):
        super().__init__(instance_id, terminal_mote_id, recipe_fingerprint)
        self._client = client

    def wait(self, timeout: float = 120.0, mode: str = "poll") -> Result:
        """Block until this run's terminal Mote commits (or fails / times out). A run
        started via ``submit`` / ``run_chain`` carries no statically-known terminal, so
        it waits for the FIRST committed Mote (the await-any path, like ``kx … --wait``)."""
        if not self._terminal:
            return self._client._await_any(self._instance, timeout)
        return self._client._await_terminal(self._instance, self._terminal, timeout, mode)

    def projection(self, at_seq: Optional[int] = None) -> "Projection":
        return self._client.get_projection(self._instance, at_seq=at_seq)

    def content(self, ref: "str | bytes") -> bytes:
        return self._client.get_content(ref, self._instance)

    def events(self, since: int = 0, follow: bool = False) -> "Iterator[Delta]":
        return self._client.stream_events(self._instance, since=since, follow=follow)

    def tokens(
        self, mote_id: "Optional[str | bytes]" = None, *, since: int = 0
    ) -> "Iterator[TokenChunk]":
        """ADVISORY per-decode token tail for ONE model mote (the committed ``result_ref``
        stays the authority — reconcile to it). Defaults to this run's terminal mote;
        a ``submit`` / ``run_chain`` run has no static terminal, so pass ``mote_id`` (or
        use :meth:`events` for the run-level delta tail)."""
        mote = mote_id if mote_id is not None else (self._terminal or None)
        if mote is None:
            raise ValueError(
                "Run.tokens() needs a mote id — this run has no statically-known terminal "
                "mote (a submit/run_chain run); pass mote_id, or use .events()"
            )
        return self._client.stream_model_tokens(self._instance, mote, since=since)

    def result(self, timeout: float = 120.0) -> Result:
        """Alias for :meth:`wait` (read as "give me the result")."""
        return self.wait(timeout=timeout)


class AsyncRun(_RunBase):
    """A started run on an :class:`~kortecx.client.AsyncKxClient`."""

    def __init__(self, client: "AsyncKxClient", instance_id, terminal_mote_id, recipe_fingerprint):
        super().__init__(instance_id, terminal_mote_id, recipe_fingerprint)
        self._client = client

    async def wait(self, timeout: float = 120.0, mode: str = "poll") -> Result:
        if not self._terminal:
            return await self._client._await_any(self._instance, timeout)
        return await self._client._await_terminal(self._instance, self._terminal, timeout, mode)

    async def projection(self, at_seq: Optional[int] = None) -> "Projection":
        return await self._client.get_projection(self._instance, at_seq=at_seq)

    async def content(self, ref: "str | bytes") -> bytes:
        return await self._client.get_content(ref, self._instance)

    def events(self, since: int = 0, follow: bool = False):
        """Return an async iterator of this run's event deltas."""
        return self._client.stream_events(self._instance, since=since, follow=follow)

    def tokens(self, mote_id: "Optional[str | bytes]" = None, *, since: int = 0):
        """Return an async iterator of ONE model mote's ADVISORY token chunks (defaults
        to the terminal mote; a ``submit`` run with no static terminal needs ``mote_id``)."""
        mote = mote_id if mote_id is not None else (self._terminal or None)
        if mote is None:
            raise ValueError(
                "AsyncRun.tokens() needs a mote id — this run has no statically-known "
                "terminal mote (a submit/run_chain run); pass mote_id, or use .events()"
            )
        return self._client.stream_model_tokens(self._instance, mote, since=since)

    async def result(self, timeout: float = 120.0) -> Result:
        return await self.wait(timeout=timeout)
