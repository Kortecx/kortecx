/**
 * A disabled control must never explain itself with a `title` alone.
 *
 * `disabled-affordance-reason.test.tsx` proves the hosted Run control does the right
 * thing. It cannot prove anything about the OTHER controls of the same shape, and
 * there were two: the App Share icon (`AppsSection`) and the workflow Share icon
 * (`WorkflowCard`), each a greyed `<Icon>` inside a span whose whole explanation was
 * `title="Sharing across parties is a Cloud capability"`. They shipped that way for
 * four sessions, and the App one did not even carry a `data-testid`, so no
 * component test could have reached it.
 *
 * A per-component test would have to be written once per site, which is how the
 * first two were missed. This scans the TREE for the marker instead, so the next
 * one is caught by existing code rather than by someone remembering.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const SRC = join(UI, "src");

/**
 * The class that styled the defect. It is deleted from `app.css`, so any
 * reappearance is either a copy of the old pattern or a revert.
 */
const RETIRED_CLASS = "iconbtn--disabled";

function sourceFiles(dir: string, acc: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      sourceFiles(full, acc);
    } else if (/\.(tsx?|css)$/.test(entry)) {
      acc.push(full);
    }
  }
  return acc;
}

function hits(needle: string): string[] {
  const out: string[] = [];
  for (const file of sourceFiles(SRC)) {
    readFileSync(file, "utf8")
      .split("\n")
      .forEach((line, i) => {
        if (line.includes(needle)) out.push(`${relative(UI, file)}:${i + 1}  ${line.trim()}`);
      });
  }
  return out;
}

/**
 * USES of a class, not mentions of it.
 *
 * A plain substring scan flagged the tombstone comment that records WHY the class
 * was retired — which would have forced the explanation to be deleted to keep the
 * guard green, i.e. the guard would have destroyed the reason it exists. So match
 * only the two forms that actually style something: the class inside a `className`
 * string, and a CSS selector opening a rule.
 */
function usageHits(cls: string): string[] {
  const inClassName = new RegExp(`className=[^>]*\\b${cls}\\b`);
  const asSelector = new RegExp(`^\\s*\\.${cls}\\b`);
  const out: string[] = [];
  for (const file of sourceFiles(SRC)) {
    readFileSync(file, "utf8")
      .split("\n")
      .forEach((line, i) => {
        if (inClassName.test(line) || asSelector.test(line)) {
          out.push(`${relative(UI, file)}:${i + 1}  ${line.trim()}`);
        }
      });
  }
  return out;
}

describe("no disabled affordance explains itself with a title alone", () => {
  it(`the retired \`${RETIRED_CLASS}\` pattern is gone from the tree`, () => {
    const found = usageHits(RETIRED_CLASS);
    expect(
      found,
      found.length === 0
        ? ""
        : `\`${RETIRED_CLASS}\` is back:\n${found.join("\n")}\n\nUse \`.affordance-off\` with an \`.affordance-off__why\` span carrying the reason as TEXT (see apps/HostedControls.tsx). A greyed icon with only a \`title\` reads as the feature being ABSENT, not unavailable.`,
    ).toEqual([]);
  });

  /**
   * The control arm. The scanner runs over a tree that is now clean, so without a
   * positive case it could be broken — a typo in the needle, a bad glob — and stay
   * green forever.
   */
  it("the scanner finds the class when it is present (it is not vacuous)", () => {
    // `.affordance-off` IS present in the tree, so scanning for it must produce
    // hits through exactly the same code path the assertion above uses.
    const found = hits("affordance-off");
    expect(found.length).toBeGreaterThan(0);
    expect(found.some((h) => h.startsWith("src/components/apps/HostedControls.tsx"))).toBe(true);
  });

  /**
   * Both Share controls now carry a reason span. Asserted by testid rather than by
   * rendering, because reaching them needs the whole Apps/Workflows data layer —
   * and the point is that the markup exists at all, which is what was missing.
   */
  it("both Share controls expose a reason span with a testid", () => {
    const apps = readFileSync(join(SRC, "components/sections/AppsSection.tsx"), "utf8");
    const workflow = readFileSync(join(SRC, "components/sections/WorkflowCard.tsx"), "utf8");
    for (const [name, src, testid] of [
      ["AppsSection", apps, "app-share-reason-"],
      ["WorkflowCard", workflow, "workflow-share-reason-"],
    ] as const) {
      expect(src, `${name} uses the .affordance-off pattern`).toContain("affordance-off");
      expect(src, `${name} exposes a reason testid`).toContain(testid);
      expect(src, `${name}'s reason is rendered text`).toContain("Sharing unavailable");
    }
  });
});
