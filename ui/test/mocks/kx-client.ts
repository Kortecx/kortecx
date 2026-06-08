/** A fake KxClientBase whose methods are vi.fn()s, for component/hook tests. */

import type { KxClientBase } from "@kortecx/sdk/web";
import { vi } from "vitest";

export interface MockClientImpl {
  listSignatures?: () => Promise<unknown>;
  getProjection?: (...args: unknown[]) => Promise<unknown>;
  invoke?: (...args: unknown[]) => Promise<unknown>;
  getContent?: (...args: unknown[]) => Promise<unknown>;
}

export function makeMockClient(impl: MockClientImpl = {}) {
  const listSignatures = vi.fn(impl.listSignatures ?? (async () => []));
  const getProjection = vi.fn(
    impl.getProjection ??
      (async () => {
        throw new Error("getProjection not stubbed");
      }),
  );
  const invoke = vi.fn(
    impl.invoke ??
      (async () => {
        throw new Error("invoke not stubbed");
      }),
  );
  const getContent = vi.fn(impl.getContent ?? (async () => new Uint8Array()));
  const close = vi.fn();
  const client = {
    listSignatures,
    getProjection,
    invoke,
    getContent,
    close,
    submitRun: vi.fn(),
    getSignature: vi.fn(),
    registerSignature: vi.fn(),
    streamEvents: vi.fn(),
    wsEvents: vi.fn(),
    endpoint: "http://127.0.0.1:50151",
  } as unknown as KxClientBase;
  return { client, listSignatures, getProjection, invoke, getContent, close };
}
