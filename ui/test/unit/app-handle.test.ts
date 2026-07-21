import { describe, expect, it } from "vitest";
import { collidingHandle, derivedHandle } from "../../src/lib/app-handle";

const apps = [{ handle: "apps/local/tip-calculator" }, { handle: "apps/local/kanban-board" }];

describe("derivedHandle", () => {
  it("derives the catalog key SaveApp will upsert on", () => {
    expect(derivedHandle("Tip Calculator")).toBe("apps/local/tip-calculator");
  });

  it("is null for a blank name — there is nothing to claim yet", () => {
    expect(derivedHandle("")).toBeNull();
    expect(derivedHandle("   ")).toBeNull();
  });

  it("trims, so a trailing space does not look like a different App", () => {
    expect(derivedHandle("  Tip Calculator  ")).toBe(derivedHandle("Tip Calculator"));
  });
});

describe("collidingHandle", () => {
  it("reports the handle an existing App already holds", () => {
    expect(collidingHandle(apps, "Tip Calculator")).toBe("apps/local/tip-calculator");
  });

  it("catches names that only LOOK different — the whole point", () => {
    // These derive the same key, so without the check the second save silently replaces
    // the first App's envelope, rails, and schedule target.
    for (const name of ["tip calculator", "TIP CALCULATOR", " Tip Calculator "]) {
      expect(collidingHandle(apps, name)).toBe("apps/local/tip-calculator");
    }
  });

  it("does NOT collide on an interior double space — that is a different handle", () => {
    // `defaultHandle` maps each invalid char to `-`, so "Tip  Calculator" becomes
    // `tip--calculator`. Pinned because it is the boundary of what the check can catch:
    // the block is exact-key, not fuzzy, and must not refuse a name that is genuinely free.
    expect(derivedHandle("Tip  Calculator")).toBe("apps/local/tip--calculator");
    expect(collidingHandle(apps, "Tip  Calculator")).toBeNull();
  });

  it("returns null for a free name", () => {
    expect(collidingHandle(apps, "Expense Tracker")).toBeNull();
  });

  it("returns null for a blank name, so an empty form shows no error", () => {
    expect(collidingHandle(apps, "")).toBeNull();
    expect(collidingHandle(apps, "  ")).toBeNull();
  });

  it("returns null when the catalog is empty or still loading", () => {
    expect(collidingHandle([], "Tip Calculator")).toBeNull();
  });
});
