/**
 * Upload bytes to the gateway's content store (`PutContent`, Batch A) — the
 * chat attach path. A CONTENT-STORE write, never a journal write; the returned
 * ref is SERVER-DERIVED blake3 (SN-8). The 32 MiB default server cap is
 * pre-checked client-side for a fast, friendly failure (the server stays the
 * fail-closed authority).
 */

import type { PutResult } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { useConnection } from "./connection-context";

/** The default server payload cap (`kx serve --content-max-bytes`). */
export const PUT_CONTENT_DEFAULT_CAP = 32 * 1024 * 1024;

export interface PutContentVars {
  readonly payload: Uint8Array;
  readonly mediaType?: string;
  readonly filename?: string;
}

export function usePutContent() {
  const { client } = useConnection();
  return useMutation<PutResult, unknown, PutContentVars>({
    mutationFn: async ({ payload, mediaType, filename }) => {
      if (!client) {
        throw new Error("not connected");
      }
      if (payload.length > PUT_CONTENT_DEFAULT_CAP) {
        throw new Error(
          `file is ${(payload.length / 1048576).toFixed(1)} MiB — over the ${
            PUT_CONTENT_DEFAULT_CAP / 1048576
          } MiB upload cap`,
        );
      }
      return client.putContent(payload, { mediaType, filename });
    },
  });
}
