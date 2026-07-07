/**
 * The browser entrypoint (`@kortecx/sdk/web`).
 *
 * Supplies a gRPC-web transport (`@connectrpc/connect-web`, fetch-based) so the
 * SAME `KxClient` surface runs in a browser — this is the dashboard's data
 * layer. Tokens are passed explicitly (no filesystem/env in a browser); the WS
 * bridge uses the `Sec-WebSocket-Protocol: bearer, <token>` subprotocol because a
 * browser cannot set an `Authorization` header on a WebSocket.
 *
 * NOTE: a browser talking gRPC-web to the gateway needs a server-side grpc-web /
 * Connect handler (a follow-up gateway PR). Until then, use {@link KxClient.wsEvents}
 * (the R5 WS bridge, which works against today's gateway) for live updates.
 */

import { createGrpcWebTransport } from "@connectrpc/connect-web";
import { KxClientBase, type KxClientOptions } from "./client.js";
import { KxUsage } from "./errors.js";
import { socketMessages } from "./events.js";
import {
  DEFAULT_ENDPOINT,
  bearerInterceptor,
  normalizeBaseUrl,
  warnIfPlaintext,
} from "./transport.js";

function resolveToken(endpoint: string, opts: KxClientOptions): string | undefined {
  if (opts.tokenFile !== undefined) {
    throw new KxUsage("tokenFile is not supported in the browser; pass `token` directly");
  }
  const token = opts.token;
  warnIfPlaintext(endpoint, token);
  return token;
}

/** A client for a running `kx serve` gateway, for browsers (gRPC-web + WS). */
export class KxClient extends KxClientBase {
  constructor(endpoint: string = DEFAULT_ENDPOINT, opts: KxClientOptions = {}) {
    const token = resolveToken(endpoint, opts);
    // (No write-size option on the fetch-based gRPC-web transport — a Batch A
    // PutContent rides a plain fetch body; the SERVER cap is the only limit.)
    const transport =
      opts.transport ??
      createGrpcWebTransport({
        baseUrl: normalizeBaseUrl(endpoint),
        interceptors: [bearerInterceptor(token)],
      });
    // Batch A: no env fallback in the browser (no `process.env`).
    super(endpoint, transport, {
      token,
      wsEndpoint: opts.wsEndpoint,
      defaultModel: opts.defaultModel,
    });
  }

  protected openWsMessages(url: string, token: string | undefined): AsyncIterable<string> {
    // Browsers cannot set an Authorization header on a WebSocket; carry the bearer
    // token in the subprotocol the gateway accepts (`ws.rs`: `bearer, <token>`).
    const protocols = token ? ["bearer", token] : undefined;
    const sock = new WebSocket(url, protocols);
    return socketMessages(
      (onMsg, onClose, onErr) => {
        sock.onmessage = (ev: MessageEvent) => {
          if (typeof ev.data === "string") onMsg(ev.data);
        };
        sock.onclose = () => onClose();
        sock.onerror = (e: Event) => onErr(e);
      },
      () => sock.close(),
    );
  }

  protected async writeOut(_path: string, _bytes: Uint8Array): Promise<void> {
    throw new KxUsage("the `out` option (write to a file) is not supported in the browser");
  }
}

export * from "./common.js";
