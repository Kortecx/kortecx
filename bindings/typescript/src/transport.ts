/**
 * Shared, platform-neutral transport helpers: endpoint/token resolution, the
 * non-loopback-plaintext warning, the bearer-token interceptor, and args
 * encoding. The actual transport CREATION (Node gRPC vs browser gRPC-web) lives
 * in `node.ts` / `web.ts` — that is the only place the two platforms differ.
 */

import type { Interceptor } from "@connectrpc/connect";
import { KxUsage } from "./errors.js";

/** The conventional gateway endpoint (matches `kx serve` / the CLI default). */
export const DEFAULT_ENDPOINT = "http://127.0.0.1:50151";

/** The arg shapes `invoke` accepts: a plain object, raw JSON bytes, or a string. */
export type Args = Record<string, unknown> | Uint8Array | string;

/**
 * True iff a bearer token would cross plaintext `http://` to a non-loopback host
 * (mirrors the CLI / Python `is_nonloopback_plaintext`).
 */
export function isNonloopbackPlaintext(endpoint: string): boolean {
  if (!endpoint.startsWith("http://")) {
    return false;
  }
  const rest = endpoint.slice("http://".length);
  let host: string;
  if (rest.startsWith("[")) {
    host = rest.slice(1).split("]", 1)[0] ?? "";
  } else {
    host = (rest.split("/", 1)[0] ?? "").split(":", 1)[0] ?? "";
  }
  return host !== "127.0.0.1" && host !== "::1" && host !== "localhost";
}

/** Ensure the base URL carries a scheme (Connect requires one) + strip a trailing slash. */
export function normalizeBaseUrl(endpoint: string): string {
  let e = endpoint.replace(/\/+$/, "");
  if (!/^https?:\/\//.test(e)) {
    e = `http://${e}`;
  }
  return e;
}

/** Warn (once, to stderr) when a token would travel in cleartext to a remote host. */
export function warnIfPlaintext(endpoint: string, token: string | undefined): void {
  if (token && isNonloopbackPlaintext(endpoint)) {
    console.warn(
      `sending a bearer token to a non-loopback plaintext endpoint (${endpoint}); it travels in cleartext — use an https:// endpoint (TLS) for remote use`,
    );
  }
}

/**
 * A Connect interceptor that adds `Authorization: Bearer <token>` to every unary
 * and streaming call (the equivalent of the Python SDK's per-call metadata). The
 * caller's party is SERVER-DERIVED from the token — the client never asserts an
 * identity (SN-8).
 */
export function bearerInterceptor(token: string | undefined): Interceptor {
  return (next) => async (req) => {
    if (token) {
      req.header.set("Authorization", `Bearer ${token}`);
    }
    return next(req);
  };
}

/** Coerce object/string/bytes args to JSON bytes, failing fast on invalid JSON. */
export function encodeArgs(args: Args): Uint8Array {
  if (args instanceof Uint8Array) {
    assertJson(new TextDecoder().decode(args));
    return args;
  }
  if (typeof args === "string") {
    assertJson(args);
    return new TextEncoder().encode(args);
  }
  if (args !== null && typeof args === "object") {
    return new TextEncoder().encode(JSON.stringify(args));
  }
  throw new KxUsage(`args must be an object, string, or Uint8Array, got ${typeof args}`);
}

function assertJson(text: string): void {
  try {
    JSON.parse(text);
  } catch (e) {
    throw new KxUsage(`args are not valid JSON: ${(e as Error).message}`);
  }
}
