/**
 * The console's navigation model — the SINGLE source of the sidebar sections.
 * Pure data (no React), so the Sidebar, the route registration, and the tests all
 * agree field-for-field. Adding a section is a one-line edit here plus its route.
 *
 * `path` MUST match a route registered in `router/router.tsx`; `icon` MUST be a key
 * in `shell/Icon.tsx`. The `nav-model` unit test pins both invariants.
 *
 * The EIGHT sections follow the product-spec IA in its order (§2.186 plan; D141.1
 * disjointness): New Chat · Workflows · Blueprints · Datasets · Tools · Context ·
 * Monitoring · Security. Display labels rename freely; ids/paths/icons stay on the
 * frozen wire-legacy handles (`chat`/`runs`/`recipes`/`systems` — route, test-ids,
 * RPC names never rename, the D136 Blueprints precedent). Activity is no longer a
 * section: it is the navbar's activity drawer. Artifacts folds into Workflows
 * (PR-2); until then the route stays reachable from a run's detail page and is
 * breadcrumb-mapped via {@link HIDDEN_SECTIONS}.
 */

export type IconName =
  | "activity"
  | "monitor"
  | "chat"
  | "runs"
  | "recipes"
  | "artifacts"
  | "context"
  | "datasets"
  | "tools"
  | "systems"
  | "settings";

/**
 * The section route paths. This is a subset of the routes registered in
 * `router/router.tsx`, so a `NavSection.path` is directly assignable to a TanStack
 * `<Link to>` (no cast) — and a typo here is a compile error, not a dead link.
 */
export type RoutePath =
  | "/activity"
  | "/monitor"
  | "/chat"
  | "/runs"
  | "/recipes"
  | "/artifacts"
  | "/context"
  | "/datasets"
  | "/tools"
  | "/systems"
  | "/settings";

export interface NavSection {
  /** Stable id (test/telemetry handle — wire-legacy, never renames). */
  readonly id: string;
  /** Sidebar label (display — renames freely). */
  readonly label: string;
  /** Route path (registered + Link-assignable). */
  readonly path: RoutePath;
  /** Icon key (must exist in Icon.tsx). */
  readonly icon: IconName;
  /** A one-line description for tooltips / collapsed-rail titles. */
  readonly hint: string;
}

/** The eight spec-IA sections, in the spec's order. */
export const NAV_SECTIONS: readonly NavSection[] = [
  {
    id: "chat",
    label: "New Chat",
    path: "/chat",
    icon: "chat",
    hint: "A fresh agentic conversation over the runtime",
  },
  {
    // Display says "Workflows"; the id/path stay on the frozen `runs` handle
    // (the PR-2 route merge adopts /workflows with redirects).
    id: "runs",
    label: "Workflows",
    path: "/runs",
    icon: "runs",
    hint: "Your runs — list, DAG & telemetry",
  },
  {
    // Display says "Blueprints" (D136); the id/path/icon stay on the frozen
    // `recipes` wire-legacy handle (route, test-ids, RPC names never rename).
    id: "recipes",
    label: "Blueprints",
    path: "/recipes",
    icon: "recipes",
    hint: "Catalog & run a blueprint",
  },
  {
    id: "datasets",
    label: "Datasets",
    path: "/datasets",
    icon: "datasets",
    hint: "RAG corpora — ingest & search",
  },
  {
    id: "tools",
    label: "Tools",
    path: "/tools",
    icon: "tools",
    hint: "MCP tool discovery & bundle preview",
  },
  {
    id: "context",
    label: "Context",
    path: "/context",
    icon: "context",
    hint: "Reusable instruction & file bundles",
  },
  {
    id: "monitor",
    label: "Monitoring",
    path: "/monitor",
    icon: "monitor",
    hint: "Gateway-wide metrics & self-correction trails",
  },
  {
    // Display says "Security"; the id/path stay on the frozen `systems` handle
    // (teams/grants viewers today; roles + policy view land with PR-8).
    id: "systems",
    label: "Security",
    path: "/systems",
    icon: "systems",
    hint: "Teams, grants & the policy view",
  },
] as const;

/**
 * Routes that are REACHABLE but not sidebar sections (deep links + breadcrumbs
 * only). Artifacts folds into Workflows at PR-2; until then a run's detail page
 * links here.
 */
export const HIDDEN_SECTIONS: readonly NavSection[] = [
  {
    id: "artifacts",
    label: "Artifacts",
    path: "/artifacts",
    icon: "artifacts",
    hint: "Committed run outputs",
  },
] as const;

/**
 * Settings is pinned shell chrome (bottom-left of the sidebar), NOT a scroll
 * section — so it lives outside {@link NAV_SECTIONS} (whose ids are pinned
 * by the nav-model unit test and iterated by the shell e2e).
 */
export const SETTINGS_SECTION: NavSection = {
  id: "settings",
  label: "Settings",
  path: "/settings",
  icon: "settings",
  hint: "Profile & console preferences",
} as const;
