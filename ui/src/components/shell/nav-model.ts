/**
 * The console's navigation model — the SINGLE source of the sidebar sections.
 * Pure data (no React), so the Sidebar, the route registration, and the tests all
 * agree field-for-field. Adding a section is a one-line edit here plus its route.
 *
 * `path` MUST match a route registered in `router/router.tsx`; `icon` MUST be a key
 * in `shell/Icon.tsx`. The `nav-model` unit test pins both invariants.
 */

export type IconName =
  | "activity"
  | "chat"
  | "runs"
  | "recipes"
  | "artifacts"
  | "datasets"
  | "systems";

/**
 * The section route paths. This is a subset of the routes registered in
 * `router/router.tsx`, so a `NavSection.path` is directly assignable to a TanStack
 * `<Link to>` (no cast) — and a typo here is a compile error, not a dead link.
 */
export type RoutePath =
  | "/activity"
  | "/chat"
  | "/runs"
  | "/recipes"
  | "/artifacts"
  | "/datasets"
  | "/systems";

export interface NavSection {
  /** Stable id (test/telemetry handle). */
  readonly id: string;
  /** Sidebar label. */
  readonly label: string;
  /** Route path (registered + Link-assignable). */
  readonly path: RoutePath;
  /** Icon key (must exist in Icon.tsx). */
  readonly icon: IconName;
  /** A one-line description for tooltips / collapsed-rail titles. */
  readonly hint: string;
}

/**
 * The six operational sections plus the agentic Chat. Activity is the dashboard
 * landing (live feed + per-run metrics + time-travel). Datasets/Systems are present
 * as destinations now; their data viewers arrive in UI-2/UI-3 (forward-compatible).
 */
export const NAV_SECTIONS: readonly NavSection[] = [
  {
    id: "activity",
    label: "Activity",
    path: "/activity",
    icon: "activity",
    hint: "Live events, metrics & time-travel",
  },
  { id: "chat", label: "Chat", path: "/chat", icon: "chat", hint: "Agentic chat over the runtime" },
  { id: "runs", label: "Runs", path: "/runs", icon: "runs", hint: "Run history (this session)" },
  {
    id: "recipes",
    label: "Recipes",
    path: "/recipes",
    icon: "recipes",
    hint: "Catalog & submit a recipe",
  },
  {
    id: "artifacts",
    label: "Artifacts",
    path: "/artifacts",
    icon: "artifacts",
    hint: "Committed run outputs",
  },
  {
    id: "datasets",
    label: "Datasets",
    path: "/datasets",
    icon: "datasets",
    hint: "RAG corpora — ingest & search",
  },
  {
    id: "systems",
    label: "Systems",
    path: "/systems",
    icon: "systems",
    hint: "Gateway, health & teams",
  },
] as const;
