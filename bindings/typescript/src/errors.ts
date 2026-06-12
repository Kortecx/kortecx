/**
 * Typed error surface for the kortecx SDK.
 *
 * Every failure is a {@link KxError} with a stable, language-independent
 * {@link ErrorCode} — so a script can branch on `err.code` without parsing a
 * message. The string values are **byte-identical** to the Python SDK's
 * `ErrorCode` and the CLI `--json` surface (the cross-SDK error contract).
 */

import { Code, ConnectError } from "@connectrpc/connect";

/**
 * A stable error code attached to every {@link KxError}.
 *
 * Consistent across the Python and TypeScript SDKs and the CLI `--json` surface.
 * Branch on these, not on human-readable messages.
 */
export enum ErrorCode {
  Unauthenticated = "unauthenticated",
  PermissionDenied = "permission_denied",
  NotFound = "not_found",
  InvalidArgument = "invalid_argument",
  Unimplemented = "unimplemented",
  Unavailable = "unavailable",
  FailedPrecondition = "failed_precondition",
  /** gRPC RESOURCE_EXHAUSTED — the live stream dropped a slow consumer. */
  CatchupRequired = "catchup_required",
  Internal = "internal",
  Connect = "connect",
  WaitTimeout = "wait_timeout",
  RunFailed = "run_failed",
  /** client-side (bad hex, invalid JSON, mutually-exclusive options). */
  Usage = "usage",
}

/** Base class for every error raised by the SDK. */
export class KxError extends Error {
  /** The stable {@link ErrorCode}. */
  readonly code: ErrorCode;
  /** The originating gRPC status name (e.g. `"PERMISSION_DENIED"`), if from the wire. */
  readonly grpcCode?: string;

  constructor(message: string, opts: { code?: ErrorCode; grpcCode?: string } = {}) {
    super(message);
    this.name = "KxError";
    this.code = opts.code ?? ErrorCode.Internal;
    this.grpcCode = opts.grpcCode;
  }

  override toString(): string {
    const base = this.message;
    return base ? `[${this.code}] ${base}` : `[${this.code}]`;
  }
}

/** No / invalid bearer token (uniform — no valid-but-unknown oracle). */
export class KxUnauthenticated extends KxError {
  constructor(message: string, opts: { grpcCode?: string } = {}) {
    super(message, { code: ErrorCode.Unauthenticated, grpcCode: opts.grpcCode });
    this.name = "KxUnauthenticated";
  }
}

/**
 * Not authorized — wrong ownership ticket, unknown handle, or no authority.
 * Uniform by design: there is no existence oracle, so a wrong `instanceId` is
 * indistinguishable from an unregistered run.
 */
export class KxPermissionDenied extends KxError {
  constructor(message: string, opts: { grpcCode?: string } = {}) {
    super(message, { code: ErrorCode.PermissionDenied, grpcCode: opts.grpcCode });
    this.name = "KxPermissionDenied";
  }
}

/** A signature (the public discovery surface) was not found. */
export class KxNotFound extends KxError {
  constructor(message: string, opts: { grpcCode?: string } = {}) {
    super(message, { code: ErrorCode.NotFound, grpcCode: opts.grpcCode });
    this.name = "KxNotFound";
  }
}

/** Server-side validation rejected the request (bad bytes, malformed args). */
export class KxInvalidArgument extends KxError {
  constructor(message: string, opts: { grpcCode?: string } = {}) {
    super(message, { code: ErrorCode.InvalidArgument, grpcCode: opts.grpcCode });
    this.name = "KxInvalidArgument";
  }
}

/** The RPC is wired in the proto but not yet served (e.g. catalog stubs). */
export class KxUnimplemented extends KxError {
  constructor(message: string, opts: { grpcCode?: string } = {}) {
    super(message, { code: ErrorCode.Unimplemented, grpcCode: opts.grpcCode });
    this.name = "KxUnimplemented";
  }
}

/** The coordinator/runtime is transiently unreachable. */
export class KxUnavailable extends KxError {
  constructor(message: string, opts: { grpcCode?: string } = {}) {
    super(message, { code: ErrorCode.Unavailable, grpcCode: opts.grpcCode });
    this.name = "KxUnavailable";
  }
}

/** A precondition failed (e.g. a refusal predicate fired, immutable conflict). */
export class KxFailedPrecondition extends KxError {
  /**
   * The structured refusal code from the `kx-refusal-code` gRPC metadata
   * (PR-2: `"R-1"`…`"R-15"` / `"D66"` / …) when the gateway refused a submit.
   * Machine-actionable — branch on this, never on the message prose.
   */
  readonly refusalCode?: string;

  constructor(message: string, opts: { grpcCode?: string; refusalCode?: string } = {}) {
    super(message, { code: ErrorCode.FailedPrecondition, grpcCode: opts.grpcCode });
    this.name = "KxFailedPrecondition";
    this.refusalCode = opts.refusalCode;
  }
}

/**
 * The live event stream dropped a slow consumer (gRPC RESOURCE_EXHAUSTED).
 * Resume a fresh `streamEvents` from {@link nextSeq} — no delta is lost or duped.
 */
export class KxCatchupRequired extends KxError {
  readonly nextSeq?: bigint;
  constructor(message: string, opts: { grpcCode?: string; nextSeq?: bigint } = {}) {
    super(message, { code: ErrorCode.CatchupRequired, grpcCode: opts.grpcCode });
    this.name = "KxCatchupRequired";
    this.nextSeq = opts.nextSeq;
  }
}

/** An internal gateway/runtime error (reachable only after authz passes). */
export class KxInternal extends KxError {
  constructor(message: string, opts: { grpcCode?: string } = {}) {
    super(message, { code: ErrorCode.Internal, grpcCode: opts.grpcCode });
    this.name = "KxInternal";
  }
}

/** The gateway endpoint could not be dialed. */
export class KxConnectError extends KxError {
  constructor(message: string, opts: { grpcCode?: string } = {}) {
    super(message, { code: ErrorCode.Connect, grpcCode: opts.grpcCode });
    this.name = "KxConnectError";
  }
}

/**
 * A `wait` timed out before the run reached a terminal state. The run is still in
 * progress and **resumable**: poll {@link instanceId} (and, for `invoke`,
 * {@link terminalMoteId}) with `getProjection` / `streamEvents`.
 */
export class KxWaitTimeout extends KxError {
  readonly instanceId?: string;
  readonly terminalMoteId?: string;
  constructor(
    message: string,
    opts: { grpcCode?: string; instanceId?: string; terminalMoteId?: string } = {},
  ) {
    super(message, { code: ErrorCode.WaitTimeout, grpcCode: opts.grpcCode });
    this.name = "KxWaitTimeout";
    this.instanceId = opts.instanceId;
    this.terminalMoteId = opts.terminalMoteId;
  }
}

/** The waited-on terminal Mote reached a failure/anomaly state. */
export class KxRunFailed extends KxError {
  readonly instanceId?: string;
  readonly terminalMoteId?: string;
  constructor(
    message: string,
    opts: { grpcCode?: string; instanceId?: string; terminalMoteId?: string } = {},
  ) {
    super(message, { code: ErrorCode.RunFailed, grpcCode: opts.grpcCode });
    this.name = "KxRunFailed";
    this.instanceId = opts.instanceId;
    this.terminalMoteId = opts.terminalMoteId;
  }
}

/** A client-side usage error: bad hex, invalid JSON, conflicting options. */
export class KxUsage extends KxError {
  constructor(message: string) {
    super(message, { code: ErrorCode.Usage });
    this.name = "KxUsage";
  }
}

/** Canonical gRPC status names, for the {@link KxError.grpcCode} field (parity
 * with the Python SDK's `status.name`). */
const CODE_NAME: Record<number, string> = {
  [Code.Canceled]: "CANCELLED",
  [Code.Unknown]: "UNKNOWN",
  [Code.InvalidArgument]: "INVALID_ARGUMENT",
  [Code.DeadlineExceeded]: "DEADLINE_EXCEEDED",
  [Code.NotFound]: "NOT_FOUND",
  [Code.AlreadyExists]: "ALREADY_EXISTS",
  [Code.PermissionDenied]: "PERMISSION_DENIED",
  [Code.ResourceExhausted]: "RESOURCE_EXHAUSTED",
  [Code.FailedPrecondition]: "FAILED_PRECONDITION",
  [Code.Aborted]: "ABORTED",
  [Code.OutOfRange]: "OUT_OF_RANGE",
  [Code.Unimplemented]: "UNIMPLEMENTED",
  [Code.Internal]: "INTERNAL",
  [Code.Unavailable]: "UNAVAILABLE",
  [Code.DataLoss]: "DATA_LOSS",
  [Code.Unauthenticated]: "UNAUTHENTICATED",
};

/**
 * Convert a raw Connect/gRPC error into the matching {@link KxError}. The mapping
 * mirrors the Python SDK's `from_rpc_error` exactly (uniform `PERMISSION_DENIED`,
 * `RESOURCE_EXHAUSTED` == catch-up-required, etc.).
 */
export function fromRpcError(err: unknown): KxError {
  if (err instanceof KxError) {
    return err;
  }
  const ce = ConnectError.from(err);
  const grpcCode = CODE_NAME[ce.code] ?? "UNKNOWN";
  const message = ce.rawMessage || grpcCode;
  switch (ce.code) {
    case Code.Unauthenticated:
      return new KxUnauthenticated(message, { grpcCode });
    case Code.PermissionDenied:
      return new KxPermissionDenied(message, { grpcCode });
    case Code.NotFound:
      return new KxNotFound(message, { grpcCode });
    case Code.InvalidArgument:
      return new KxInvalidArgument(message, { grpcCode });
    case Code.Unimplemented:
      return new KxUnimplemented(message, { grpcCode });
    case Code.Unavailable:
      return new KxUnavailable(message, { grpcCode });
    case Code.FailedPrecondition:
      return new KxFailedPrecondition(message, {
        grpcCode,
        // Connect merges trailers into `metadata` on both grpc and grpc-web
        // transports — the PR-2 structured refusal code rides there.
        refusalCode: ce.metadata.get("kx-refusal-code") ?? undefined,
      });
    case Code.ResourceExhausted:
      return new KxCatchupRequired(message, { grpcCode });
    case Code.Internal:
      return new KxInternal(message, { grpcCode });
    default:
      return new KxError(message, { grpcCode });
  }
}

/** Await a unary RPC, translating any Connect/gRPC error into a typed {@link KxError}. */
export async function rpc<T>(p: Promise<T>): Promise<T> {
  try {
    return await p;
  } catch (e) {
    throw fromRpcError(e);
  }
}
