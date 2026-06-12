/** A fake KxClientBase whose methods are vi.fn()s, for component/hook tests. */

import type { KxClientBase } from "@kortecx/sdk/web";
import { vi } from "vitest";

export interface MockClientImpl {
  listSignatures?: () => Promise<unknown>;
  getProjection?: (...args: unknown[]) => Promise<unknown>;
  invoke?: (...args: unknown[]) => Promise<unknown>;
  getContent?: (...args: unknown[]) => Promise<unknown>;
  getSignature?: (...args: unknown[]) => Promise<unknown>;
  /** An async-iterable of `Delta`s (the live WS tail). */
  wsEvents?: (...args: unknown[]) => AsyncIterable<unknown>;
  listRuns?: (...args: unknown[]) => Promise<unknown>;
  listRecipes?: (...args: unknown[]) => Promise<unknown>;
  getRecipeForm?: (...args: unknown[]) => Promise<unknown>;
  listTeams?: (...args: unknown[]) => Promise<unknown>;
  listTeamMembers?: (...args: unknown[]) => Promise<unknown>;
  listAssetGrants?: (...args: unknown[]) => Promise<unknown>;
  listReplanRounds?: (...args: unknown[]) => Promise<unknown>;
  listReactTurns?: (...args: unknown[]) => Promise<unknown>;
  listCaptureRecords?: (...args: unknown[]) => Promise<unknown>;
  getMoteDetail?: (...args: unknown[]) => Promise<unknown>;
  getContentBatch?: (...args: unknown[]) => Promise<unknown>;
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
  const getSignature = vi.fn(impl.getSignature ?? (async () => new Uint8Array()));
  // Default: an empty, immediately-ending stream.
  const wsEvents = vi.fn(
    impl.wsEvents ??
      async function* () {
        /* no events */
      },
  );
  const listRuns = vi.fn(impl.listRuns ?? (async () => ({ runs: [], hasMore: false })));
  const listRecipes = vi.fn(impl.listRecipes ?? (async () => []));
  const getRecipeForm = vi.fn(
    impl.getRecipeForm ??
      (async () => {
        throw new Error("getRecipeForm not stubbed");
      }),
  );
  const listTeams = vi.fn(impl.listTeams ?? (async () => []));
  const listTeamMembers = vi.fn(impl.listTeamMembers ?? (async () => ({ owner: "", members: [] })));
  const listAssetGrants = vi.fn(impl.listAssetGrants ?? (async () => ({ owner: "", grants: [] })));
  const listReplanRounds = vi.fn(
    impl.listReplanRounds ?? (async () => ({ rounds: [], hasMore: false })),
  );
  const listReactTurns = vi.fn(
    impl.listReactTurns ?? (async () => ({ turns: [], hasMore: false })),
  );
  const listCaptureRecords = vi.fn(
    impl.listCaptureRecords ?? (async () => ({ records: [], hasMore: false })),
  );
  const getMoteDetail = vi.fn(
    impl.getMoteDetail ??
      (async () => {
        throw new Error("getMoteDetail not stubbed");
      }),
  );
  const getContentBatch = vi.fn(impl.getContentBatch ?? (async () => []));
  const close = vi.fn();
  const client = {
    listSignatures,
    getProjection,
    invoke,
    getContent,
    getSignature,
    wsEvents,
    listRuns,
    listRecipes,
    getRecipeForm,
    listTeams,
    listTeamMembers,
    listAssetGrants,
    listReplanRounds,
    listReactTurns,
    listCaptureRecords,
    getMoteDetail,
    getContentBatch,
    close,
    submitRun: vi.fn(),
    registerSignature: vi.fn(),
    streamEvents: vi.fn(),
    endpoint: "http://127.0.0.1:50151",
  } as unknown as KxClientBase;
  return {
    client,
    listSignatures,
    getProjection,
    invoke,
    getContent,
    getSignature,
    wsEvents,
    listRuns,
    listRecipes,
    getRecipeForm,
    listTeams,
    listTeamMembers,
    listAssetGrants,
    listReplanRounds,
    listReactTurns,
    listCaptureRecords,
    getMoteDetail,
    getContentBatch,
    close,
  };
}
