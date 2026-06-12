"""Typed error surface for the kortecx SDK.

Every failure is a :class:`KxError` with a stable, language-independent
:class:`ErrorCode` (so a script can branch on ``err.code`` without parsing a
message), mapped from the gateway's gRPC status. The mapping is exact — it
mirrors the per-RPC status codes the gateway returns (uniform ``PERMISSION_DENIED``
with no existence oracle; ``RESOURCE_EXHAUSTED`` == catch-up-required; etc.).
"""

from __future__ import annotations

from enum import Enum
from typing import Optional

try:  # grpc is a hard dependency; this import is defensive for type-only contexts.
    import grpc
except Exception:  # pragma: no cover
    grpc = None


class ErrorCode(str, Enum):
    """A stable error code attached to every :class:`KxError`.

    These are part of the SDK contract — consistent across the Python and
    TypeScript SDKs and the CLI ``--json`` surface. Branch on these, not on
    human-readable messages.
    """

    UNAUTHENTICATED = "unauthenticated"
    PERMISSION_DENIED = "permission_denied"
    NOT_FOUND = "not_found"
    INVALID_ARGUMENT = "invalid_argument"
    UNIMPLEMENTED = "unimplemented"
    UNAVAILABLE = "unavailable"
    FAILED_PRECONDITION = "failed_precondition"
    CATCHUP_REQUIRED = "catchup_required"  # gRPC RESOURCE_EXHAUSTED
    INTERNAL = "internal"
    CONNECT = "connect"
    WAIT_TIMEOUT = "wait_timeout"
    RUN_FAILED = "run_failed"
    USAGE = "usage"  # client-side (bad hex, invalid JSON, mutually-exclusive flags)


class KxError(Exception):
    """Base class for every error raised by the SDK.

    Attributes
    ----------
    code:
        The stable :class:`ErrorCode`.
    grpc_code:
        The originating gRPC status code name (e.g. ``"PERMISSION_DENIED"``),
        when the error came from the wire; ``None`` for client-side errors.
    """

    code: ErrorCode = ErrorCode.INTERNAL

    def __init__(
        self,
        message: str,
        *,
        code: Optional[ErrorCode] = None,
        grpc_code: Optional[str] = None,
    ) -> None:
        super().__init__(message)
        if code is not None:
            self.code = code
        self.grpc_code = grpc_code

    def __str__(self) -> str:  # pragma: no cover - trivial
        base = super().__str__()
        return f"[{self.code.value}] {base}" if base else f"[{self.code.value}]"


class KxUnauthenticated(KxError):
    """No / invalid bearer token (uniform — no valid-but-unknown oracle)."""

    code = ErrorCode.UNAUTHENTICATED


class KxPermissionDenied(KxError):
    """Not authorized — wrong ownership ticket, unknown handle, or no authority.

    Uniform by design: there is no existence oracle, so a wrong ``instance_id``
    is indistinguishable from an unregistered run.
    """

    code = ErrorCode.PERMISSION_DENIED


class KxNotFound(KxError):
    """A signature (the public discovery surface) was not found."""

    code = ErrorCode.NOT_FOUND


class KxInvalidArgument(KxError):
    """Server-side validation rejected the request (bad bytes, malformed args)."""

    code = ErrorCode.INVALID_ARGUMENT


class KxUnimplemented(KxError):
    """The RPC is wired in the proto but not yet served (e.g. catalog stubs)."""

    code = ErrorCode.UNIMPLEMENTED


class KxUnavailable(KxError):
    """The coordinator/runtime is transiently unreachable."""

    code = ErrorCode.UNAVAILABLE


class KxFailedPrecondition(KxError):
    """A precondition failed (e.g. a refusal predicate fired, immutable conflict).

    Attributes
    ----------
    refusal_code:
        The structured refusal code from the ``kx-refusal-code`` gRPC metadata
        (PR-2: ``"R-1"``…``"R-15"`` / ``"D66"`` / …) when the gateway refused a
        submit. Machine-actionable — branch on this, never on the prose.
    """

    code = ErrorCode.FAILED_PRECONDITION

    def __init__(self, message: str, *, refusal_code: Optional[str] = None, **kw: object) -> None:
        super().__init__(message, **kw)  # type: ignore[arg-type]
        self.refusal_code = refusal_code


class KxCatchupRequired(KxError):
    """The live event stream dropped a slow consumer (gRPC RESOURCE_EXHAUSTED).

    Resume a fresh ``stream_events`` from :attr:`next_seq` — no delta is lost or
    duplicated.
    """

    code = ErrorCode.CATCHUP_REQUIRED

    def __init__(self, message: str, *, next_seq: Optional[int] = None, **kw: object) -> None:
        super().__init__(message, **kw)  # type: ignore[arg-type]
        self.next_seq = next_seq


class KxInternal(KxError):
    """An internal gateway/runtime error (reachable only after authz passes)."""

    code = ErrorCode.INTERNAL


class KxConnectError(KxError):
    """The gateway endpoint could not be dialed."""

    code = ErrorCode.CONNECT


class KxWaitTimeout(KxError):
    """A ``wait`` timed out before the run reached a terminal state.

    The run is still in progress and **resumable**: poll :attr:`instance_id` (and,
    for ``invoke``, :attr:`terminal_mote_id`) with ``get_projection`` / ``events``.
    """

    code = ErrorCode.WAIT_TIMEOUT

    def __init__(
        self,
        message: str,
        *,
        instance_id: Optional[str] = None,
        terminal_mote_id: Optional[str] = None,
        **kw: object,
    ) -> None:
        super().__init__(message, **kw)  # type: ignore[arg-type]
        self.instance_id = instance_id
        self.terminal_mote_id = terminal_mote_id


class KxRunFailed(KxError):
    """The waited-on terminal Mote reached a failure/anomaly state."""

    code = ErrorCode.RUN_FAILED

    def __init__(
        self,
        message: str,
        *,
        instance_id: Optional[str] = None,
        terminal_mote_id: Optional[str] = None,
        **kw: object,
    ) -> None:
        super().__init__(message, **kw)  # type: ignore[arg-type]
        self.instance_id = instance_id
        self.terminal_mote_id = terminal_mote_id


class KxUsage(KxError):
    """A client-side usage error: bad hex, invalid JSON, conflicting options."""

    code = ErrorCode.USAGE


def _status_map() -> dict:
    """Map gRPC status codes → SDK exception classes. Lazily built (grpc import)."""
    if grpc is None:  # pragma: no cover
        return {}
    return {
        grpc.StatusCode.UNAUTHENTICATED: KxUnauthenticated,
        grpc.StatusCode.PERMISSION_DENIED: KxPermissionDenied,
        grpc.StatusCode.NOT_FOUND: KxNotFound,
        grpc.StatusCode.INVALID_ARGUMENT: KxInvalidArgument,
        grpc.StatusCode.UNIMPLEMENTED: KxUnimplemented,
        grpc.StatusCode.UNAVAILABLE: KxUnavailable,
        grpc.StatusCode.FAILED_PRECONDITION: KxFailedPrecondition,
        grpc.StatusCode.RESOURCE_EXHAUSTED: KxCatchupRequired,
        grpc.StatusCode.INTERNAL: KxInternal,
    }


def from_rpc_error(err: "grpc.RpcError") -> KxError:
    """Convert a raw gRPC error (sync or aio) into the matching :class:`KxError`.

    ``UNAVAILABLE`` is surfaced as :class:`KxUnavailable`; a connect failure (which
    grpc also reports as ``UNAVAILABLE`` at dial time) is handled separately by the
    client as :class:`KxConnectError`.
    """
    try:
        status = err.code()
        details = err.details()
    except Exception:  # pragma: no cover - non-status RpcError
        return KxError(str(err))
    message = details or (status.name if status is not None else "rpc error")
    cls = _status_map().get(status, KxError)
    grpc_code = status.name if status is not None else None
    if cls is KxFailedPrecondition:
        return KxFailedPrecondition(message, grpc_code=grpc_code, refusal_code=_refusal_code(err))
    return cls(message, grpc_code=grpc_code)


def _refusal_code(err: "grpc.RpcError") -> Optional[str]:
    """Extract the PR-2 ``kx-refusal-code`` trailer (sync ``grpc.Call`` and
    ``aio.AioRpcError`` both expose ``trailing_metadata()``; defensive — an
    error without trailers simply has no code)."""
    try:
        trailers = err.trailing_metadata()
    except Exception:  # pragma: no cover - non-call RpcError
        return None
    if trailers is None:
        return None
    for key, value in trailers:
        if key == "kx-refusal-code":
            return value if isinstance(value, str) else value.decode("ascii", "replace")
    return None
