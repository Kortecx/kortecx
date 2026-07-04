/**
 * Query keys are ENDPOINT-SCOPED so switching gateways never serves a stale
 * cross-endpoint cache. `atSeq` distinguishes a pinned time-travel snapshot from
 * the live ("live") view.
 */

export const queryKeys = {
  signatures: (endpoint: string) => ["kx", endpoint, "signatures"] as const,
  projection: (endpoint: string, instanceId: string, atSeq?: number) =>
    ["kx", endpoint, "projection", instanceId, atSeq ?? "live"] as const,
  /** A committed artifact blob, scoped by its owning run + content ref. */
  content: (endpoint: string, instanceId: string, ref: string) =>
    ["kx", endpoint, "content", instanceId, ref] as const,
  /** Durable run enumeration (`ListRuns`), scoped by the requested page size. */
  runs: (endpoint: string, limit: number) => ["kx", endpoint, "runs", limit] as const,
  /** The invocable recipe catalog (`ListRecipes`). */
  recipes: (endpoint: string) => ["kx", endpoint, "recipes"] as const,
  /** The fingerprint→handle naming map (`ListRecipes` summaries, PR-2.1). */
  recipeNames: (endpoint: string) => ["kx", endpoint, "recipe-names"] as const,
  /** The handle→summary metadata map (`ListRecipes` summaries, PR-4.1b cards). */
  recipeSummaries: (endpoint: string) => ["kx", endpoint, "recipe-summaries"] as const,
  /** One recipe's free-param form (`GetRecipeForm`), scoped by handle. */
  recipeForm: (endpoint: string, handle: string) =>
    ["kx", endpoint, "recipe-form", handle] as const,
  /** A run's captured Invoke args (`GetRunInputs`, PR-D "Re-run with changes"). */
  runInputs: (endpoint: string, instanceId: string) =>
    ["kx", endpoint, "run-inputs", instanceId] as const,
  /** The teams the gateway knows (`ListTeams`). */
  teams: (endpoint: string) => ["kx", endpoint, "teams"] as const,
  /** One team's members (`ListTeamMembers`); `assetRef` distinguishes the resolved view. */
  teamMembers: (endpoint: string, teamId: string, assetRef?: string) =>
    ["kx", endpoint, "team-members", teamId, assetRef ?? "none"] as const,
  /** The active grants on an asset (`ListAssetGrants`), scoped by asset ref. */
  assetGrants: (endpoint: string, assetRef: string) =>
    ["kx", endpoint, "asset-grants", assetRef] as const,
  /** The advisory tool manifests (`ListToolManifests`) — display-only, never authority. */
  toolManifests: (endpoint: string) => ["kx", endpoint, "tool-manifests"] as const,
  /** The durable tools-registry inventory (`DiscoverTools`, PR-6a) — the governance
   *  view (what is registered, with what authority). Registration grants none (SN-8). */
  discoverTools: (endpoint: string) => ["kx", endpoint, "discover-tools"] as const,
  /** The registered external MCP servers (`ListMcpServers`, PR-6b-1) — the live
   *  Connections govern surface. Server-derived ids; credentials by NAME only. */
  mcpServers: (endpoint: string) => ["kx", endpoint, "mcp-servers"] as const,
  /** The host secret-store NAMES (`ListSecretNames`, MM-3 / D110) — names + audit
   *  timestamps only; the secret VALUE is write-only (never on this key, D81). */
  secretNames: (endpoint: string, limit: number) =>
    ["kx", endpoint, "secret-names", limit] as const,
  /** The registered triggers (`ListTriggers`, D113 / D170.b) — the govern view.
   *  `authSecretPresent` reports only whether a ref NAME is attached (never a value). */
  triggers: (endpoint: string, limit: number) => ["kx", endpoint, "triggers", limit] as const,
  /** This party's context bundles (`ListContextBundles`, PR-7) — named,
   *  content-addressed grounding. Caller-scoped; `bundleRef` is server-derived (SN-8). */
  contextBundles: (endpoint: string) => ["kx", endpoint, "context-bundles"] as const,
  /** This party's POC-4 Apps (`ListApps`) — durable kortecx.app/v1 envelopes.
   *  Caller-scoped; `appRef` is server-derived (SN-8). */
  apps: (endpoint: string) => ["kx", endpoint, "apps"] as const,
  /** This party's RC-SW1 skills (`ListSkills`) — declarative kortecx.skill/v1
   *  bundles (instructions + tool grant-WISHES; a wish is never a grant). */
  skills: (endpoint: string) => ["kx", endpoint, "skills"] as const,
  /** One skill's form (`GetSkillForm`) — wishes + the ADVISORY registered bit. */
  skillForm: (endpoint: string, name: string) => ["kx", endpoint, "skill", name] as const,
  /** One App's full envelope (`GetApp`), keyed by handle. */
  app: (endpoint: string, handle: string) => ["kx", endpoint, "app", handle] as const,
  /** One App's project branch manifest (`GetBranch`, POC-5d), keyed by handle
   *  (one-App-one-branch ⇒ the branch handle IS the App handle). */
  appBranch: (endpoint: string, handle: string) => ["kx", endpoint, "app-branch", handle] as const,
  /** One App project file's FULL body (`GetBranchContent`, POC-5d). Keyed by the
   *  content ref — content-addressed ⇒ immutable (cache forever). */
  appFileContent: (endpoint: string, handle: string, contentRef: string) =>
    ["kx", endpoint, "app-file", handle, contentRef] as const,
  /** An App's live scaffold status (`GetScaffoldStatus`, POC-5a), keyed by the
   *  project branch handle. Short-poll while the scaffold is active. */
  scaffoldStatus: (endpoint: string, branchHandle: string) =>
    ["kx", endpoint, "scaffold-status", branchHandle] as const,
  /** A context-item's FULL body (POC-2 view/edit, uploads-scope `GetContent`),
   *  keyed by its content ref — content-addressed ⇒ immutable (cache forever). */
  contextItemBody: (endpoint: string, contentRef: string) =>
    ["kx", endpoint, "context-item-body", contentRef] as const,
  /** This party's D155 branches (`ListBranches`) — content-addressed file branches.
   *  Caller-scoped; `branchRef` is server-derived (SN-8). */
  branches: (endpoint: string) => ["kx", endpoint, "branches"] as const,
  /** The datasets (RAG corpora) the gateway holds (`ListDatasets`). */
  datasets: (endpoint: string) => ["kx", endpoint, "datasets"] as const,
  /** A dataset query (`QueryDataset`), scoped by dataset + query text + k. */
  datasetQuery: (endpoint: string, dataset: string, text: string, k: number) =>
    ["kx", endpoint, "dataset-query", dataset, text, k] as const,
  /** Advisory fuzzy discovery (`FuzzyDiscovery`), scoped by dataset + text + k. */
  fuzzyDiscovery: (endpoint: string, dataset: string, text: string, k: number) =>
    ["kx", endpoint, "fuzzy-discovery", dataset, text, k] as const,
  /** RC5a: the durable agentic memories (`ListMemories`, episodic log). */
  memories: (endpoint: string, includeTombstoned = false) =>
    ["kx", endpoint, "memories", includeTombstoned] as const,
  /** RC5a: a memory recall (`RecallMemory`), scoped by query text + k. */
  memoryRecall: (endpoint: string, text: string, k: number) =>
    ["kx", endpoint, "memory-recall", text, k] as const,
  /** RC5b: namespace memory statistics (`MemoryStats`). */
  memoryStats: (endpoint: string) => ["kx", endpoint, "memory-stats"] as const,
  /** RC5b: a decay preview (`DecayMemory` dry-run), scoped by policy. */
  memoryDecay: (endpoint: string, ttlDays: number, minAccess: number) =>
    ["kx", endpoint, "memory-decay", ttlDays, minAccess] as const,
  /** Gateway liveness probe (endpoint-scoped). */
  health: (endpoint: string) => ["kx", endpoint, "health"] as const,
  /** POC-1 Settings "Workspace": the non-secret server configuration (`GetServerInfo`). */
  serverInfo: (endpoint: string) => ["kx", endpoint, "server-info"] as const,
  /** The re-plan-round trail (`ListReplanRounds`), scoped by page size. */
  replanRounds: (endpoint: string, limit: number) =>
    ["kx", endpoint, "replan-rounds", limit] as const,
  /** The re-rank-turn trail (`ListReRankTurns`, RC4c-2), scoped by page size. */
  rerankTurns: (endpoint: string, limit: number) =>
    ["kx", endpoint, "rerank-turns", limit] as const,
  /** The ReAct-turn trail (`ListReactTurns`); `instanceId` scopes to one run,
   *  `chainSalt` (PR-R1) to one chain within it (serve's shared journal). */
  reactTurns: (
    endpoint: string,
    instanceId: string | undefined,
    limit: number,
    chainSalt?: string,
  ) => ["kx", endpoint, "react-turns", instanceId ?? "all", chainSalt ?? "all", limit] as const,
  /** The capture-record stream (`ListCaptureRecords`); `instanceId` scopes to one run. */
  captureRecords: (endpoint: string, instanceId: string | undefined, limit: number) =>
    ["kx", endpoint, "capture-records", instanceId ?? "all", limit] as const,
  /** The per-run quality readout (`ScoreRun`, RC1/D172); scoped to one run. */
  evalScore: (endpoint: string, instanceId: string) =>
    ["kx", endpoint, "eval-score", instanceId] as const,
  /** The discoverable models (`ListModels`) — display-only (SN-8). */
  models: (endpoint: string) => ["kx", endpoint, "models"] as const,
  /** A batched content fetch (`GetContentBatch`), scoped by run + a stable refs key.
   *  Content-addressed ⇒ immutable (cache forever). `scope` = instanceId or "uploads". */
  contentBatch: (endpoint: string, scope: string, refsKey: string) =>
    ["kx", endpoint, "content-batch", scope, refsKey] as const,
  /** One Mote's admitted definition (`GetMoteDetail`, Batch B). Keyed by the
   *  COMMITTED def hash — content-addressed ⇒ immutable (cache forever). */
  moteDetail: (endpoint: string, instanceId: string, moteId: string, defHash: string) =>
    ["kx", endpoint, "mote-detail", instanceId, moteId, defHash] as const,
  /** The mote execution-telemetry pages (`ListMoteTelemetry`, Batch C);
   *  `instanceId` scopes to one run; cursor pages live inside the one key. */
  telemetry: (endpoint: string, instanceId: string | undefined, pageSize: number) =>
    ["kx", endpoint, "telemetry", instanceId ?? "all", pageSize] as const,
  /** The exact per-model token-economy rollup (`ListTelemetrySummary`, W1a-3);
   *  `instanceId` scopes to one run (else all runs). One unary call, no cursor. */
  telemetrySummary: (endpoint: string, instanceId: string | undefined) =>
    ["kx", endpoint, "telemetry-summary", instanceId ?? "all"] as const,
  /** The operator alerts inbox pages (`ListAlerts`, W1a-2); terminal-failure
   *  facts folded newest-first; cursor pages live inside the one key. */
  alerts: (endpoint: string, instanceId: string | undefined, pageSize: number) =>
    ["kx", endpoint, "alerts", instanceId ?? "all", pageSize] as const,
  /** RC6a: the pending HITL approvals inbox (`ListPendingApprovals`, D114) — the
   *  govern surface over withheld world-mutating actions. Polled (approvals are NOT
   *  on the event stream); `limit` scopes the page. `requestId` is server-derived
   *  (SN-8); a grant/deny releases a staged action, never mints a warrant. */
  pendingApprovals: (endpoint: string, limit: number) =>
    ["kx", endpoint, "pending-approvals", limit] as const,
  /** RC6a: a run's local spend estimate (`GetRunCost`, M11) — display-only over the
   *  durable turn/tool counters at operator-set rates (a guardrail, not billing). */
  runCost: (endpoint: string, instanceId: string) =>
    ["kx", endpoint, "run-cost", instanceId] as const,
};
