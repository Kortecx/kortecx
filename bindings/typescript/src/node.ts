/**
 * The Node.js entrypoint (`@kortecx/sdk` / `@kortecx/sdk/node`).
 *
 * Supplies a real gRPC transport over HTTP/2 (`@connectrpc/connect-node`) — it
 * talks to today's `kx serve` (tonic) unchanged — plus file/env token resolution
 * and a `ws`-backed WebSocket bridge. The browser equivalent is `@kortecx/sdk/web`.
 */

import { readFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { createGrpcTransport } from "@connectrpc/connect-node";
import { KxClientBase, type KxClientOptions } from "./client.js";
import { KxUsage } from "./errors.js";
import { socketMessages } from "./events.js";
import {
  DEFAULT_ENDPOINT,
  bearerInterceptor,
  normalizeBaseUrl,
  warnIfPlaintext,
} from "./transport.js";

const READ_MAX_BYTES = 0x40000000; // 1 GiB — accommodate large committed results.
const WRITE_MAX_BYTES = 0x04000000; // 64 MiB — headroom over the default 32 MiB PutContent cap.

function resolveToken(endpoint: string, opts: KxClientOptions): string | undefined {
  if (opts.token !== undefined && opts.tokenFile !== undefined) {
    throw new KxUsage("token and tokenFile are mutually exclusive");
  }
  let resolved: string | undefined;
  if (opts.tokenFile !== undefined) {
    resolved = readFileSync(opts.tokenFile, "utf-8").trim();
    if (!resolved) {
      throw new KxUsage(`tokenFile ${opts.tokenFile} is empty`);
    }
  } else if (opts.token !== undefined) {
    resolved = opts.token;
  } else {
    const env = process.env.KX_TOKEN;
    resolved = env ? env.trim() : undefined;
  }
  warnIfPlaintext(endpoint, resolved);
  return resolved;
}

/** A synchronous-to-construct client for a running `kx serve` gateway (Node). */
export class KxClient extends KxClientBase {
  constructor(endpoint: string = DEFAULT_ENDPOINT, opts: KxClientOptions = {}) {
    const token = resolveToken(endpoint, opts);
    const transport =
      opts.transport ??
      createGrpcTransport({
        baseUrl: normalizeBaseUrl(endpoint),
        interceptors: [bearerInterceptor(token)],
        readMaxBytes: READ_MAX_BYTES,
        writeMaxBytes: WRITE_MAX_BYTES,
      });
    // Batch A: an explicit defaultModel wins over the KX_DEFAULT_MODEL env fallback.
    const defaultModel = opts.defaultModel ?? process.env.KX_DEFAULT_MODEL ?? "";
    super(endpoint, transport, { token, wsEndpoint: opts.wsEndpoint, defaultModel });
  }

  protected async *openWsMessages(url: string, token: string | undefined): AsyncIterable<string> {
    let WebSocketImpl: typeof import("ws").WebSocket;
    try {
      WebSocketImpl = (await import("ws")).WebSocket;
    } catch {
      throw new KxUsage("the WebSocket events client needs the 'ws' package: npm install ws");
    }
    const headers = token ? { Authorization: `Bearer ${token}` } : undefined;
    const sock = new WebSocketImpl(url, { headers });
    yield* socketMessages(
      (onMsg, onClose, onErr) => {
        sock.on("message", (data: { toString(): string }) => onMsg(data.toString()));
        sock.on("close", onClose);
        sock.on("error", onErr);
      },
      () => sock.close(),
    );
  }

  protected async writeOut(path: string, bytes: Uint8Array): Promise<void> {
    await writeFile(path, bytes);
  }
}

export * from "./common.js";
