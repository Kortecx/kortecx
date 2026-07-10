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
  /** An async-iterable of `GlobalDelta`s (the Batch C global tail). */
  wsAllEvents?: (...args: unknown[]) => AsyncIterable<unknown>;
  listMoteTelemetry?: (...args: unknown[]) => Promise<unknown>;
  listRuns?: (...args: unknown[]) => Promise<unknown>;
  listRecipes?: (...args: unknown[]) => Promise<unknown>;
  getRecipeForm?: (...args: unknown[]) => Promise<unknown>;
  listTeams?: (...args: unknown[]) => Promise<unknown>;
  listTeamMembers?: (...args: unknown[]) => Promise<unknown>;
  listAssetGrants?: (...args: unknown[]) => Promise<unknown>;
  listReplanRounds?: (...args: unknown[]) => Promise<unknown>;
  listReactTurns?: (...args: unknown[]) => Promise<unknown>;
  listRerankTurns?: (...args: unknown[]) => Promise<unknown>;
  listCaptureRecords?: (...args: unknown[]) => Promise<unknown>;
  getMoteDetail?: (...args: unknown[]) => Promise<unknown>;
  getContentBatch?: (...args: unknown[]) => Promise<unknown>;
  listModels?: (...args: unknown[]) => Promise<unknown>;
  loadModel?: (...args: unknown[]) => Promise<unknown>;
  offloadModel?: (...args: unknown[]) => Promise<unknown>;
  getServerInfo?: (...args: unknown[]) => Promise<unknown>;
  pullModel?: (...args: unknown[]) => Promise<unknown>;
  getPullStatus?: (...args: unknown[]) => Promise<unknown>;
  setActiveModel?: (...args: unknown[]) => Promise<unknown>;
  listContextBundles?: (...args: unknown[]) => Promise<unknown>;
  putContextBundle?: (...args: unknown[]) => Promise<unknown>;
  deleteContextBundle?: (...args: unknown[]) => Promise<unknown>;
  getContextBundle?: (...args: unknown[]) => Promise<unknown>;
  editContextItem?: (...args: unknown[]) => Promise<unknown>;
  removeContextItem?: (...args: unknown[]) => Promise<unknown>;
  // MM-3 / D110: the host secret store (`client.secrets.*`). The value is write-only.
  secretsList?: (...args: unknown[]) => Promise<unknown>;
  secretsSet?: (...args: unknown[]) => Promise<unknown>;
  secretsRemove?: (...args: unknown[]) => Promise<unknown>;
  // D113 / D170.b: the trigger registry (`client.triggers.*`).
  triggersAdd?: (...args: unknown[]) => Promise<unknown>;
  triggersList?: (...args: unknown[]) => Promise<unknown>;
  triggersTest?: (...args: unknown[]) => Promise<unknown>;
  triggersFire?: (...args: unknown[]) => Promise<unknown>;
  triggersRemove?: (...args: unknown[]) => Promise<unknown>;
  // D114 / RC6a: the HITL approvals inbox (`client.approvals.*`) + cost readout (`client.cost.*`).
  approvalsListPending?: (...args: unknown[]) => Promise<unknown>;
  approvalsGrant?: (...args: unknown[]) => Promise<unknown>;
  approvalsDeny?: (...args: unknown[]) => Promise<unknown>;
  costGetRunCost?: (...args: unknown[]) => Promise<unknown>;
  // PR-6b-1 / RC6a: the external-MCP-gateway connection methods (flat on the client;
  // surfaced read-only in Monitoring's connector-health panel).
  listMcpServers?: (...args: unknown[]) => Promise<unknown>;
  testMcpServer?: (...args: unknown[]) => Promise<unknown>;
  deregisterMcpServer?: (...args: unknown[]) => Promise<unknown>;
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
  const listRerankTurns = vi.fn(
    impl.listRerankTurns ?? (async () => ({ turns: [], hasMore: false })),
  );
  const listCaptureRecords = vi.fn(
    impl.listCaptureRecords ?? (async () => ({ records: [], hasMore: false })),
  );
  // Default: an empty, immediately-ending global stream.
  const wsAllEvents = vi.fn(
    impl.wsAllEvents ??
      async function* () {
        /* no events */
      },
  );
  const listMoteTelemetry = vi.fn(
    impl.listMoteTelemetry ?? (async () => ({ rows: [], hasMore: false })),
  );
  const getMoteDetail = vi.fn(
    impl.getMoteDetail ??
      (async () => {
        throw new Error("getMoteDetail not stubbed");
      }),
  );
  const getContentBatch = vi.fn(impl.getContentBatch ?? (async () => []));
  const listModels = vi.fn(impl.listModels ?? (async () => []));
  const loadModel = vi.fn(
    impl.loadModel ?? (async (modelId: string) => ({ modelId, loaded: true, wasResident: false })),
  );
  const offloadModel = vi.fn(
    impl.offloadModel ??
      (async (modelId: string) => ({ modelId, loaded: false, wasResident: true })),
  );
  // Model Control v2: downloads OFF by default (deny-by-default); a test opts in.
  const getServerInfo = vi.fn(
    impl.getServerInfo ?? (async () => ({ allowModelPull: false, activeModelId: "" })),
  );
  const pullModel = vi.fn(impl.pullModel ?? (async () => "pulled-model"));
  const getPullStatus = vi.fn(
    impl.getPullStatus ??
      (async (modelId: string) => ({
        modelId,
        phase: "done",
        bytesDownloaded: 0,
        bytesTotal: 0,
        detail: "",
      })),
  );
  const setActiveModel = vi.fn(impl.setActiveModel ?? (async (modelId: string) => modelId));
  const listContextBundles = vi.fn(impl.listContextBundles ?? (async () => []));
  const putContextBundle = vi.fn(
    impl.putContextBundle ??
      (async () => ({ bundleRef: "ab".repeat(8), handle: "", deduplicated: false })),
  );
  const deleteContextBundle = vi.fn(impl.deleteContextBundle ?? (async () => true));
  const getContextBundle = vi.fn(impl.getContextBundle ?? (async () => null));
  const putResult = { bundleRef: "ef".repeat(8), handle: "", deduplicated: false };
  const editContextItem = vi.fn(impl.editContextItem ?? (async () => putResult));
  const removeContextItem = vi.fn(impl.removeContextItem ?? (async () => putResult));
  // MM-3 / D110: the host secret store namespace (`client.secrets.*`).
  const secretsList = vi.fn(impl.secretsList ?? (async () => ({ names: [], hasMore: false })));
  const secretsSet = vi.fn(impl.secretsSet ?? (async () => true));
  const secretsRemove = vi.fn(impl.secretsRemove ?? (async () => true));
  // D113 / D170.b: the trigger registry namespace (`client.triggers.*`).
  const triggersAdd = vi.fn(impl.triggersAdd ?? (async () => ({ triggerId: "ab".repeat(8) })));
  const triggersList = vi.fn(impl.triggersList ?? (async () => ({ triggers: [], hasMore: false })));
  const triggersTest = vi.fn(impl.triggersTest ?? (async () => ({ ok: true, detail: "" })));
  const triggersFire = vi.fn(
    impl.triggersFire ?? (async () => ({ instanceId: "cd".repeat(8), deduped: false })),
  );
  const triggersRemove = vi.fn(impl.triggersRemove ?? (async () => true));
  // D114 / RC6a: the HITL approvals inbox + the cost readout namespaces.
  const approvalsListPending = vi.fn(
    impl.approvalsListPending ?? (async () => ({ approvals: [] })),
  );
  const approvalsGrant = vi.fn(impl.approvalsGrant ?? (async () => true));
  const approvalsDeny = vi.fn(impl.approvalsDeny ?? (async () => true));
  const costGetRunCost = vi.fn(
    impl.costGetRunCost ??
      (async () => ({
        instanceId: "",
        turns: 0,
        toolCalls: 0,
        estimatedMicroUsd: 0,
        ceilingMicroUsd: 0,
        perTurnMicroUsd: 0,
        perToolCallMicroUsd: 0,
        overCeiling: false,
      })),
  );
  const listMcpServers = vi.fn(
    impl.listMcpServers ?? (async () => ({ servers: [], hasMore: false })),
  );
  const testMcpServer = vi.fn(impl.testMcpServer ?? (async () => true));
  const deregisterMcpServer = vi.fn(impl.deregisterMcpServer ?? (async () => true));
  const close = vi.fn();
  const client = {
    listSignatures,
    getProjection,
    invoke,
    getContent,
    getSignature,
    wsEvents,
    wsAllEvents,
    listMoteTelemetry,
    listRuns,
    listRecipes,
    getRecipeForm,
    listTeams,
    listTeamMembers,
    listAssetGrants,
    listReplanRounds,
    listReactTurns,
    listRerankTurns,
    listCaptureRecords,
    getMoteDetail,
    getContentBatch,
    listModels,
    loadModel,
    offloadModel,
    getServerInfo,
    pullModel,
    getPullStatus,
    setActiveModel,
    listContextBundles,
    putContextBundle,
    deleteContextBundle,
    getContextBundle,
    editContextItem,
    removeContextItem,
    secrets: { list: secretsList, set: secretsSet, remove: secretsRemove },
    triggers: {
      add: triggersAdd,
      list: triggersList,
      test: triggersTest,
      fire: triggersFire,
      remove: triggersRemove,
    },
    approvals: {
      listPending: approvalsListPending,
      grant: approvalsGrant,
      deny: approvalsDeny,
    },
    cost: { getRunCost: costGetRunCost },
    listMcpServers,
    testMcpServer,
    deregisterMcpServer,
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
    listRerankTurns,
    listCaptureRecords,
    getMoteDetail,
    getContentBatch,
    listModels,
    loadModel,
    offloadModel,
    getServerInfo,
    pullModel,
    getPullStatus,
    setActiveModel,
    listContextBundles,
    putContextBundle,
    deleteContextBundle,
    getContextBundle,
    editContextItem,
    removeContextItem,
    secretsList,
    secretsSet,
    secretsRemove,
    triggersAdd,
    triggersList,
    triggersTest,
    triggersFire,
    triggersRemove,
    approvalsListPending,
    approvalsGrant,
    approvalsDeny,
    costGetRunCost,
    listMcpServers,
    testMcpServer,
    deregisterMcpServer,
    close,
  };
}
