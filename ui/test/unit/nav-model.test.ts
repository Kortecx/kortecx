import { describe, expect, it } from "vitest";
import {
  HIDDEN_SECTIONS,
  NAV_SECTIONS,
  SETTINGS_SECTION,
} from "../../src/components/shell/nav-model";

describe("NAV_SECTIONS (POC-5c / D168 flat IA)", () => {
  it("has the eight flat sections in the D168 order", () => {
    expect(NAV_SECTIONS.map((s) => s.id)).toEqual([
      "chat",
      "apps",
      "runs",
      "context",
      "tools",
      "models",
      "monitor",
      "systems",
    ]);
  });

  it("display labels rename; ids/icons stay on the frozen wire-legacy handles", () => {
    const byId = new Map(NAV_SECTIONS.map((s) => [s.id, s]));
    // The spec IA renames (D141): chat → New Chat, runs → Workflows, systems →
    // Security — ids/icons untouched. PR-2 moved the `runs` section's ROUTE to
    // /workflows (the D141.1 merge; /runs redirects).
    expect(byId.get("chat")?.label).toBe("New Chat");
    expect(byId.get("chat")?.path).toBe("/chat");
    expect(byId.get("runs")?.label).toBe("Workflows");
    expect(byId.get("runs")?.path).toBe("/workflows");
    expect(byId.get("runs")?.icon).toBe("runs");
    expect(byId.get("systems")?.label).toBe("Security");
    expect(byId.get("systems")?.path).toBe("/systems");
    // The Tools section is the Integrations hub (Tools/Connections/Triggers/Secrets);
    // the id/path/icon stay on the frozen `tools` wire-legacy handle.
    expect(byId.get("tools")?.label).toBe("Integrations");
    expect(byId.get("tools")?.path).toBe("/tools");
    expect(byId.get("tools")?.icon).toBe("tools");
    // The Context section is the data umbrella (Bundles + the Datasets tab).
    expect(byId.get("context")?.label).toBe("Context");
    expect(byId.get("models")?.path).toBe("/models");
    expect(byId.get("models")?.icon).toBe("models");
  });

  it("the demoted sections are NOT flat nav buttons (folded into a section/tab)", () => {
    const ids = new Set(NAV_SECTIONS.map((s) => s.id));
    for (const demoted of ["dashboard", "recipes", "datasets", "branches", "policies"]) {
      expect(ids.has(demoted), `${demoted} should not be a flat nav section`).toBe(false);
    }
  });

  it("Activity is NOT a section (it is the navbar drawer)", () => {
    expect(NAV_SECTIONS.some((s) => s.id === "activity")).toBe(false);
  });

  it("ids and paths are unique across nav + hidden + settings", () => {
    const all = [...NAV_SECTIONS, ...HIDDEN_SECTIONS, SETTINGS_SECTION];
    expect(new Set(all.map((s) => s.id)).size).toBe(all.length);
    expect(new Set(all.map((s) => s.path)).size).toBe(all.length);
  });

  it("every path is absolute and every section has a label/hint/icon", () => {
    for (const s of [...NAV_SECTIONS, ...HIDDEN_SECTIONS]) {
      expect(s.path.startsWith("/")).toBe(true);
      expect(s.label.length).toBeGreaterThan(0);
      expect(s.hint.length).toBeGreaterThan(0);
      expect(s.icon.length).toBeGreaterThan(0);
    }
  });
});

describe("HIDDEN_SECTIONS (POC-5c: demoted-but-reachable — deep link + breadcrumb + ⌘K)", () => {
  it("holds the five demoted sections so no capability disappears (D168 no-regression)", () => {
    expect(HIDDEN_SECTIONS.map((s) => s.id)).toEqual([
      "dashboard",
      "recipes",
      "datasets",
      "branches",
      "policies",
    ]);
  });

  it("keeps the demoted sections' frozen routes (deep links stay valid)", () => {
    const byId = new Map(HIDDEN_SECTIONS.map((s) => [s.id, s]));
    expect(byId.get("recipes")?.path).toBe("/recipes");
    expect(byId.get("recipes")?.label).toBe("Blueprints");
    expect(byId.get("datasets")?.path).toBe("/datasets");
    expect(byId.get("branches")?.path).toBe("/branches");
    expect(byId.get("policies")?.path).toBe("/policies");
    expect(byId.get("dashboard")?.path).toBe("/dashboard");
  });

  it("none of the demoted sections appear in NAV_SECTIONS", () => {
    const nav = new Set(NAV_SECTIONS.map((s) => s.id));
    for (const s of HIDDEN_SECTIONS) {
      expect(nav.has(s.id)).toBe(false);
    }
  });
});

describe("SETTINGS_SECTION", () => {
  it("is pinned shell chrome, not a ninth nav section", () => {
    expect(SETTINGS_SECTION.id).toBe("settings");
    expect(SETTINGS_SECTION.path).toBe("/settings");
    expect(SETTINGS_SECTION.icon).toBe("settings");
    expect(NAV_SECTIONS.some((s) => s.id === SETTINGS_SECTION.id)).toBe(false);
  });
});
