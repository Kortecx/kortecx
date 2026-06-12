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
 * Monitoring · Security. Display labels rename freely; section IDS/icons stay on
 * the frozen wire-legacy handles (`chat`/`runs`/`recipes`/`systems` — test-ids,
 * RPC names never rename, the D136 Blueprints precedent). Activity is no longer a
 * section: it is the navbar's activity drawer. PR-2 completed the D141.1 route
 * merge: the `runs` section LIVES at `/workflows` (old `/runs`, `/runs/$id`,
 * `/artifacts`, `/activity` deep links redirect there) and Artifacts is a TAB of
 * a run's detail page — one capability, one home.
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
  | "/monitor"
  | "/chat"
  | "/workflows"
  | "/recipes"
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
    // Display says "Workflows" and the route merged to /workflows (PR-2,
    // D141.1); the id/icon stay on the frozen `runs` handle (test-ids,
    // telemetry never rename) and /runs redirects here.
    id: "runs",
    label: "Workflows",
    path: "/workflows",
    icon: "runs",
    hint: "Your runs — list, DAG, artifacts & telemetry",
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
 * only). EMPTY since PR-2 folded Artifacts into the Workflows run-detail tabs
 * (D141.1 — one capability, one home); the const stays so the breadcrumb/nav
 * APIs keep one shape when a future hidden route lands.
 */
export const HIDDEN_SECTIONS: readonly NavSection[] = [] as const;

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
