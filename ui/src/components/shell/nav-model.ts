/**
 * The console's navigation model — the SINGLE source of the sidebar sections.
 * Pure data (no React), so the Sidebar, the route registration, and the tests all
 * agree field-for-field. Adding a section is a one-line edit here plus its route.
 *
 * `path` MUST match a route registered in `router/router.tsx`; `icon` MUST be a key
 * in `shell/Icon.tsx`. The `nav-model` unit test pins both invariants.
 *
 * POC-5c (D168): the console is a flat set of plain-button sections — New Chat · Apps ·
 * Workflows · Context · Integrations · Models · Security — with NO sidebar groups and
 * NO "Coming" placeholders. Display labels rename freely; section IDS/icons stay on the
 * frozen wire-legacy handles (`chat`/`runs`/`recipes`/`systems` — test-ids, RPC names
 * never rename, the D136 Blueprints precedent). The section moves fold the demoted
 * sections in WITHOUT losing any capability: Datasets → a Context tab, run history +
 * self-correction trails → Workflows tabs, Blueprints → the Workflows catalog, Branches
 * → the App window, HITL approvals + per-App policies → Apps. Their ROUTES stay
 * registered and remain breadcrumb- + ⌘K-reachable via {@link HIDDEN_SECTIONS}.
 * Activity is the navbar drawer (not a section); `/` redirects to Chat (D137 — New Chat
 * is the landing).
 */

export type IconName =
  | "activity"
  | "chat"
  | "runs"
  | "recipes"
  | "artifacts"
  | "context"
  | "branches"
  | "datasets"
  | "tools"
  | "models"
  | "systems"
  | "settings";

/**
 * The section route paths. This is a subset of the routes registered in
 * `router/router.tsx`, so a `NavSection.path` is directly assignable to a TanStack
 * `<Link to>` (no cast) — and a typo here is a compile error, not a dead link. The
 * demoted-section paths stay in the union: they are no longer sidebar buttons but
 * are still reachable (deep link, breadcrumb, ⌘K) via {@link HIDDEN_SECTIONS}.
 */
export type RoutePath =
  | "/chat"
  | "/apps"
  | "/workflows"
  | "/recipes"
  | "/context"
  | "/branches"
  | "/datasets"
  | "/tools"
  | "/models"
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

/** The flat sections, in the D168 order. New Chat is the landing (D137); `/`
 *  redirects to Chat, unchanged. No groups, no placeholders. */
export const NAV_SECTIONS: readonly NavSection[] = [
  {
    id: "chat",
    label: "New Chat",
    path: "/chat",
    icon: "chat",
    hint: "A fresh agentic conversation over the runtime",
  },
  {
    // POC-4/5: durable, reusable Apps (kortecx.app/v1 envelopes) — author with the
    // SDK/CLI, browse + run + open here. Subsumes the Workflows/Blueprints catalog
    // conceptually; the agentic scaffold + in-CAS editing shipped in POC-5. Reuses
    // the `artifacts` glyph (the prior DEV_PLACEHOLDERS icon).
    id: "apps",
    label: "Apps",
    path: "/apps",
    icon: "artifacts",
    hint: "Durable, reusable Apps — browse, run & open",
  },
  {
    // POC-5c: Workflows is the runnable CATALOG — browse a blueprint and trigger a
    // single run — plus your own run history and the self-correction trails (its Runs
    // and Trails tabs). The id/icon stay on the frozen `runs` handle (test-ids,
    // telemetry never rename); /runs redirects.
    id: "runs",
    label: "Workflows",
    path: "/workflows",
    icon: "runs",
    hint: "Browse blueprints & trigger a run",
  },
  {
    // POC-5c: Context is the data + storage umbrella — reusable instruction/file
    // bundles (bundles.db) PLUS the Datasets tab (datasets.db, RAG corpora). A UI
    // umbrella over TWO SEPARATE stores; no backend merge.
    id: "context",
    label: "Context",
    path: "/context",
    icon: "context",
    hint: "Reusable bundles & RAG datasets",
  },
  {
    // Display says "Integrations" (the Tools/Connections/Triggers/Secrets hub); the
    // id/path/icon stay on the frozen `tools` wire-legacy handle (test-ids, route,
    // telemetry never rename — the D136 Blueprints precedent).
    id: "tools",
    label: "Integrations",
    path: "/tools",
    icon: "tools",
    hint: "Tools, MCP connections, triggers & secrets",
  },
  {
    // A read-only view over the models serving this gateway (`ListModels`) plus the
    // client-local default-model pick (POC-5c). Listing a model never routes one
    // (SN-8); FFI-free serves return an honest empty list.
    id: "models",
    label: "Models",
    path: "/models",
    icon: "models",
    hint: "Models serving this gateway — pick the default",
  },
  {
    // Display says "Security"; the id/path stay on the frozen `systems` handle. The
    // single-user capability-manifest surface — an App's resolved reach, capability
    // ceiling, and model route.
    id: "systems",
    label: "Security",
    path: "/systems",
    icon: "systems",
    hint: "App capability manifests — reach, ceiling & model route",
  },
] as const;

/**
 * Routes that are REACHABLE but not sidebar sections (deep links, breadcrumbs and
 * ⌘K only). POC-5c (D168): the demoted sections live here so nothing exposed
 * disappears — each is folded into a flat section's tab/catalog (Datasets → Context,
 * Blueprints → Workflows, Branches → the App window, Policies → Apps) yet keeps its own
 * route + breadcrumb + ⌘K jump-to. The data is verbatim from the pre-flat NAV_SECTIONS
 * (frozen ids/paths/icons).
 */
export const HIDDEN_SECTIONS: readonly NavSection[] = [
  {
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
    id: "branches",
    label: "Branches",
    path: "/branches",
    icon: "branches",
    hint: "Snapshot & edit files as content-addressed branches",
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
