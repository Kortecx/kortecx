import { describe, expect, it } from "vitest";
import {
  CLOUD_PLACEHOLDERS,
  DEV_PLACEHOLDERS,
  HIDDEN_SECTIONS,
  NAV_GROUPS,
  NAV_SECTIONS,
  SETTINGS_SECTION,
  type SectionColor,
} from "../../src/components/shell/nav-model";

describe("NAV_SECTIONS", () => {
  it("has the spec-IA sections in the spec's order (Dashboard leads — PR-C1/D150)", () => {
    expect(NAV_SECTIONS.map((s) => s.id)).toEqual([
      "dashboard",
      "chat",
      "apps",
      "runs",
      "recipes",
      "datasets",
      "tools",
      "models",
      "context",
      "branches",
      "monitor",
      "systems",
      "policies",
    ]);
  });

  it("the Dashboard landing keeps Chat as the default route (D137 unchanged)", () => {
    const byId = new Map(NAV_SECTIONS.map((s) => [s.id, s]));
    expect(byId.get("dashboard")?.label).toBe("Dashboard");
    expect(byId.get("dashboard")?.path).toBe("/dashboard");
    expect(byId.get("dashboard")?.icon).toBe("activity");
  });

  it("display labels rename; ids/icons stay on the frozen wire-legacy handles", () => {
    const byId = new Map(NAV_SECTIONS.map((s) => [s.id, s]));
    // D136 precedent: recipes displays Blueprints.
    expect(byId.get("recipes")?.label).toBe("Blueprints");
    expect(byId.get("recipes")?.path).toBe("/recipes");
    expect(byId.get("recipes")?.icon).toBe("recipes");
    // The spec IA renames (§2.186 / D141): chat → New Chat, runs → Workflows,
    // systems → Security — ids/icons untouched. PR-2 moved the `runs`
    // section's ROUTE to /workflows (the D141.1 merge; /runs redirects).
    expect(byId.get("chat")?.label).toBe("New Chat");
    expect(byId.get("chat")?.path).toBe("/chat");
    expect(byId.get("runs")?.label).toBe("Workflows");
    expect(byId.get("runs")?.path).toBe("/workflows");
    expect(byId.get("runs")?.icon).toBe("runs");
    expect(byId.get("systems")?.label).toBe("Security");
    expect(byId.get("systems")?.path).toBe("/systems");
  });

  it("Policies is a REAL section (promoted from a placeholder in POC-5b)", () => {
    const byId = new Map(NAV_SECTIONS.map((s) => [s.id, s]));
    expect(byId.get("policies")?.label).toBe("Policies");
    expect(byId.get("policies")?.path).toBe("/policies");
    expect(byId.get("policies")?.icon).toBe("systems");
    // Grouped under Security (the agent-write policy gate).
    const security = NAV_GROUPS.find((g) => g.id === "security");
    expect(security?.sectionIds).toContain("policies");
  });

  it("Activity is NOT a section (it is the navbar drawer)", () => {
    expect(NAV_SECTIONS.some((s) => s.id === "activity")).toBe(false);
  });

  it("the Models section (PR-C2) is a read-only Tools-group view at /models", () => {
    const byId = new Map(NAV_SECTIONS.map((s) => [s.id, s]));
    expect(byId.get("models")?.label).toBe("Models");
    expect(byId.get("models")?.path).toBe("/models");
    expect(byId.get("models")?.icon).toBe("models");
    const tools = NAV_GROUPS.find((g) => g.id === "tools");
    expect(tools?.sectionIds).toContain("models");
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

describe("HIDDEN_SECTIONS", () => {
  it("is empty since PR-2 folded Artifacts into the Workflows tabs (D141.1)", () => {
    expect(HIDDEN_SECTIONS).toEqual([]);
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

describe("NAV_GROUPS (PR-B / D150 sidebar grouping)", () => {
  const COLORS: readonly SectionColor[] = [
    "warning",
    "teal",
    "violet",
    "error",
    "success",
    "neutral",
  ];

  it("groups in the user-decided order", () => {
    expect(NAV_GROUPS.map((g) => g.id)).toEqual([
      "workspace",
      "data",
      "tools",
      "monitoring",
      "security",
    ]);
  });

  it("every grouped section id references a real NAV_SECTIONS section", () => {
    const ids = new Set(NAV_SECTIONS.map((s) => s.id));
    for (const g of NAV_GROUPS) {
      for (const id of g.sectionIds) {
        expect(ids.has(id), `group ${g.id} references unknown section ${id}`).toBe(true);
      }
    }
  });

  it("partitions NAV_SECTIONS exactly once (none dropped, none duplicated)", () => {
    const grouped = NAV_GROUPS.flatMap((g) => g.sectionIds);
    expect(grouped.length).toBe(NAV_SECTIONS.length);
    expect(new Set(grouped).size).toBe(grouped.length);
    expect(new Set(grouped)).toEqual(new Set(NAV_SECTIONS.map((s) => s.id)));
  });

  it("uses only allowed section colours", () => {
    for (const g of NAV_GROUPS) {
      expect(COLORS).toContain(g.color);
    }
  });
});

describe("CLOUD_PLACEHOLDERS (honest-disabled — GR15/D129)", () => {
  it("are never navigable: NO path field on any placeholder", () => {
    for (const p of CLOUD_PLACEHOLDERS) {
      expect(p).not.toHaveProperty("path");
      expect(p.label.length).toBeGreaterThan(0);
      expect(p.icon.length).toBeGreaterThan(0);
    }
  });

  it("do not collide with real section ids", () => {
    const sectionIds = new Set(NAV_SECTIONS.map((s) => s.id));
    for (const p of CLOUD_PLACEHOLDERS) {
      expect(sectionIds.has(p.id)).toBe(false);
    }
  });
});

describe("DEV_PLACEHOLDERS (honest in-development — GR15/≈D166)", () => {
  it("are never navigable: NO path field on any placeholder", () => {
    for (const p of DEV_PLACEHOLDERS) {
      expect(p).not.toHaveProperty("path");
      expect(p.label.length).toBeGreaterThan(0);
      expect(p.icon.length).toBeGreaterThan(0);
    }
  });

  it("is EMPTY now (Apps promoted POC-4, Policies promoted POC-5b)", () => {
    expect(DEV_PLACEHOLDERS).toEqual([]);
  });

  it("Apps + Policies are now REAL sections (promoted from placeholders)", () => {
    expect(DEV_PLACEHOLDERS.some((p) => p.id === "apps")).toBe(false);
    expect(DEV_PLACEHOLDERS.some((p) => p.id === "policies")).toBe(false);
    expect(NAV_SECTIONS.some((s) => s.id === "apps" && s.path === "/apps")).toBe(true);
    expect(NAV_SECTIONS.some((s) => s.id === "policies" && s.path === "/policies")).toBe(true);
  });

  it("do not collide with real section ids or the cloud placeholders", () => {
    const taken = new Set([
      ...NAV_SECTIONS.map((s) => s.id),
      ...CLOUD_PLACEHOLDERS.map((p) => p.id),
    ]);
    for (const p of DEV_PLACEHOLDERS) {
      expect(taken.has(p.id)).toBe(false);
    }
  });
});
