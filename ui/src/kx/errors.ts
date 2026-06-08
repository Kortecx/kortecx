/**
 * Map any thrown error (a typed SDK {@link KxError} or a raw Connect/gRPC error)
 * into a UI-facing shape the components render. Branch on the stable `ErrorCode`,
 * never on a human message (the SDK guarantees the codes are cross-surface stable).
 */

import { ErrorCode, fromRpcError } from "@kortecx/sdk/web";

export type UiErrorKind =
  | "reauth"
  | "forbidden"
  | "not-found"
  | "not-wired"
  | "bad-input"
  | "retry"
  | "generic";

export interface UiError {
  readonly code: string;
  readonly kind: UiErrorKind;
  readonly title: string;
  readonly message: string;
  readonly retryable: boolean;
}

/**
 * Read the stable error code by DUCK-TYPING `.code` rather than `instanceof`. The
 * SDK bundles its `web` and `node` entry points standalone, so the `KxError` class
 * identity differs across entry points — `instanceof` would silently miss. Any
 * SDK error (web or node) carries a string `.code`; a raw Connect/gRPC error is
 * normalized through `fromRpcError`.
 */
function errorCode(err: unknown): string {
  if (err !== null && typeof err === "object" && "code" in err) {
    const c = (err as { code?: unknown }).code;
    if (typeof c === "string") {
      return c;
    }
  }
  return fromRpcError(err).code;
}

export function toUiError(err: unknown): UiError {
  const code = errorCode(err);
  const message = err instanceof Error ? err.message : String(err);
  switch (code) {
    case ErrorCode.Unauthenticated:
      return {
        code,
        kind: "reauth",
        title: "Authentication required",
        message: message || "This gateway requires a valid bearer token.",
        retryable: false,
      };
    case ErrorCode.PermissionDenied:
      // Uniform by design — no existence oracle (a wrong instance id and an
      // unauthorized one are indistinguishable). Say so honestly.
      return {
        code,
        kind: "forbidden",
        title: "Not found or not authorized",
        message: message || "This run does not exist, or this token cannot access it.",
        retryable: false,
      };
    case ErrorCode.NotFound:
      return {
        code,
        kind: "not-found",
        title: "Not found",
        message: message || "No such resource.",
        retryable: false,
      };
    case ErrorCode.Unimplemented:
      return {
        code,
        kind: "not-wired",
        title: "Not available here",
        message: message || "This capability is not wired on the connected gateway.",
        retryable: false,
      };
    case ErrorCode.InvalidArgument:
    case ErrorCode.Usage:
    case ErrorCode.FailedPrecondition:
      return {
        code,
        kind: "bad-input",
        title: "Invalid request",
        message: message || "The request was rejected as invalid.",
        retryable: false,
      };
    case ErrorCode.Unavailable:
    case ErrorCode.Connect:
    case ErrorCode.CatchupRequired:
    case ErrorCode.WaitTimeout:
      return {
        code,
        kind: "retry",
        title: "Gateway unreachable",
        message:
          message || "Could not reach the gateway — check the endpoint and that `kx serve` is up.",
        retryable: true,
      };
    case ErrorCode.RunFailed:
      return {
        code,
        kind: "generic",
        title: "Run failed",
        message: message || "The run's terminal Mote failed.",
        retryable: false,
      };
    default:
      return {
        code,
        kind: "generic",
        title: "Something went wrong",
        message: message || "Unexpected error.",
        retryable: true,
      };
  }
}
