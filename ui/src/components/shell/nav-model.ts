/**
 * The console's navigation model — the SINGLE source of the sidebar sections.
 * Pure data (no React), so the Sidebar, the route registration, and the tests all
 * agree field-for-field. Adding a section is a one-line edit here plus its route.
 *
 * `path` MUST match a route registered in `router/router.tsx`; `icon` MUST be a key
 * in `shell/Icon.tsx`. The `nav-model` unit test pins both invariants.
 *
 * The core sections follow the product-spec IA in its order (§2.186 plan; D141.1
 * disjointness): New Chat · Workflows · Blueprints · Datasets · Tools · Context ·
 * Monitoring · Security, plus the Dashboard landing (PR-C1, D150) leading and the
 * read-only Models view (PR-C2, D152) in the Tools group. Display labels rename
 * freely; section IDS/icons stay on the frozen wire-legacy handles
 * (`chat`/`runs`/`recipes`/`systems` — test-ids, RPC names never rename, the D136
 * Blueprints precedent). Activity is no longer a section: it is the navbar's
 * activity drawer. PR-2 completed the D141.1 route
 * merge: the `runs` section LIVES at `/workflows` (old `/runs`, `/runs/$id`,
 * `/artifacts`, `/activity` deep links redirect there) and Artifacts is a TAB of
 * a run's detail page — one capability, one home.
 */

import type { Glyph } from "./Icon";

export type IconName =
  | "activity"
  | "monitor"
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
 * `<Link to>` (no cast) — and a typo here is a compile error, not a dead link.
 */
export type RoutePath =
  | "/dashboard"
  | "/monitor"
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
  | "/policies"
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

/** The spec-IA sections, in the spec's order. The Dashboard landing (PR-C1, D150)
 *  leads the Workspace group; `/` still redirects to Chat (D137 — unchanged). */
export const NAV_SECTIONS: readonly NavSection[] = [
  {
    id: "dashboard",
    label: "Dashboard",
    path: "/dashboard",
    icon: "activity",
    hint: "A live at-a-glance overview of this gateway",
  },
  {
    id: "chat",
    label: "New Chat",
    path: "/chat",
    icon: "chat",
    hint: "A fresh agentic conversation over the runtime",
  },
  {
    // POC-4: durable, reusable Apps (kortecx.app/v1 envelopes) — author with the
    // SDK/CLI, browse + run here (read-only). Subsumes the Workflows/Blueprints
    // catalog conceptually but is ADDITIVE for now (nothing exposed disappears);
    // the agentic scaffold + in-CAS editing arrive in POC-5a. Reuses the
    // `artifacts` glyph (the prior DEV_PLACEHOLDERS icon).
    id: "apps",
    label: "Apps",
    path: "/apps",
    icon: "artifacts",
    hint: "Durable, reusable Apps — browse & run",
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
    // A read-only view over the models serving this gateway (`ListModels`).
    // Display-only (SN-8): listing a model never routes one. FFI-free serves
    // return an honest empty list; an old gateway degrades to "not wired".
    id: "models",
    label: "Models",
    path: "/models",
    icon: "models",
    hint: "Discover the models serving this gateway",
  },
  {
    id: "context",
    label: "Context",
    path: "/context",
    icon: "context",
    hint: "Reusable instruction & file bundles",
  },
  {
    // D155: content-addressed branches over operator-approved host files
    // (snapshot into CAS; edit in-CAS). Default-OFF (KX_SERVE_FS_ROOT); the host
    // is never written in this phase. Degrades to an honest disabled state.
    id: "branches",
    label: "Branches",
    path: "/branches",
    icon: "branches",
    hint: "Snapshot & edit files as content-addressed branches",
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
  {
    // POC-5b: the per-App lock surface — the OSS agent-write policy gate. A locked
    // App refuses agentic in-CAS edits at the advance() chokepoint. Promoted from
    // the DEV_PLACEHOLDERS "Coming" group in POC-5b.
    id: "policies",
    label: "Policies",
    path: "/policies",
    icon: "systems",
    hint: "Per-App locks — the agent-write policy gate",
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

/**
 * Section-grouping colour (PR-B reference-app adoption, D150). A TOKEN NAME, never
 * a raw hex — `app.css` owns the per-theme flip, so the AA-lock stays the single
 * source of truth. Maps 1:1 to a `--{color}` palette token (`neutral` → `--text-3`).
 */
export type SectionColor = "warning" | "teal" | "violet" | "error" | "success" | "neutral";

/**
 * A sidebar GROUP — a presentation layer OVER {@link NAV_SECTIONS} (which stays the
 * single source of section identity). `sectionIds` reference NAV_SECTIONS ids in
 * render order; the nav-model test asserts every section is grouped exactly once,
 * so a new section can never silently fall out of the sidebar.
 */
export interface NavGroup {
  /** Stable group id (test handle). */
  readonly id: string;
  /** Uppercase display label (renames freely). */
  readonly label: string;
  /** The group's accent colour (a palette token name). */
  readonly color: SectionColor;
  /** Section ids INTO {@link NAV_SECTIONS}, in render order. */
  readonly sectionIds: readonly string[];
}

/**
 * The sidebar groups (D150 — user-decided mapping of the eight REAL sections). The
 * flat {@link NAV_SECTIONS} order is UNCHANGED (its unit-test assertion + flat
 * consumers stay green); this drives the sidebar's grouped render order only.
 */
export const NAV_GROUPS: readonly NavGroup[] = [
  {
    id: "workspace",
    label: "Workspace",
    color: "warning",
    sectionIds: ["dashboard", "chat", "apps", "runs", "recipes"],
  },
  { id: "data", label: "Data", color: "teal", sectionIds: ["datasets", "context", "branches"] },
  { id: "tools", label: "Tools", color: "violet", sectionIds: ["tools", "models"] },
  { id: "monitoring", label: "Monitoring", color: "error", sectionIds: ["monitor"] },
  { id: "security", label: "Security", color: "success", sectionIds: ["systems", "policies"] },
] as const;

/**
 * An HONEST disabled "Cloud" placeholder (GR15 don't-fake-gaps + D129 cloud line).
 * Has NO `path` — it is NEVER navigable, rendered greyed with a "Cloud" chip. These
 * map to our ACTUAL planned managed-cloud capabilities (D118 permissioned
 * federation · D129 managed multi-party), so the group is honest about what arrives
 * in Cloud rather than fabricating a local feature.
 */
export interface CloudPlaceholder {
  readonly id: string;
  readonly label: string;
  readonly icon: Glyph;
}

export const CLOUD_GROUP_LABEL = "Cloud";

export const CLOUD_PLACEHOLDERS: readonly CloudPlaceholder[] = [
  { id: "sharing", label: "Sharing", icon: "share" },
  { id: "federation", label: "Federation", icon: "systems" },
  { id: "experts", label: "Experts", icon: "activity" },
] as const;

/**
 * An HONEST in-development placeholder (GR15 don't-fake-gaps): a section on the POC
 * roadmap (≈D166) that is NOT yet built. Structurally a {@link CloudPlaceholder} — NO
 * `path`, NEVER navigable, rendered greyed with an "In dev" chip — so the nav previews
 * what's coming without fabricating a working surface. Promote one to a real
 * {@link NAV_SECTIONS} entry in the PR that ships it. EMPTY now: Apps was promoted in
 * POC-4 and Policies (the per-App agent-write gate) in POC-5b; the const stays so the
 * "Coming" group keeps one shape when the next un-built POC section is previewed.
 */
export type DevPlaceholder = CloudPlaceholder;

export const DEV_GROUP_LABEL = "Coming";

export const DEV_PLACEHOLDERS: readonly DevPlaceholder[] = [] as const;
