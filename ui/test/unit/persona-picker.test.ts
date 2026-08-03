/**
 * The persona swap logic.
 *
 * A persona's text is PREPENDED to a step's prompt and the pair is identity-bearing
 * — the same persona + task always compile to the same recipe fingerprint. So the
 * failure that matters is silent: picking twice must SWAP the role, not stack a
 * second one on top. A stacked prompt still runs, still looks plausible in the
 * editor, and quietly changes the step's identity.
 *
 * Fixtures are built from `PERSONAS` itself rather than pinned strings — a pinned
 * role text would compare the code to a copy of itself and pass after the catalog
 * changed underneath it.
 */

import { PERSONAS, personaNames } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { activePersona, promptBody, withPersona } from "../../src/components/builder/PersonaPicker";

const NAMES = personaNames();

/**
 * The precondition, asserted at module load and FAILING rather than skipping.
 * "Nothing stacked" is also true of a catalog with one persona in it, so a suite
 * that quietly adapted to a one-entry catalog would keep passing while proving
 * nothing about swapping.
 */
function nth(i: number): string {
  const name = NAMES[i];
  if (name === undefined || !PERSONAS[name]) {
    throw new Error(
      `the persona catalog needs at least ${i + 1} entries with role text for these tests ` +
        `to mean anything; got ${JSON.stringify(NAMES)}`,
    );
  }
  return name;
}

const FIRST = nth(0);
const SECOND = nth(1);
/** The role text for a name the catalog is known to carry. */
const role = (name: string): string => PERSONAS[name] ?? "";
const TASK = "Summarise the overnight issues by theme.";

describe("the persona applied to a step's prompt", () => {
  it("the catalog has at least two personas, or these tests prove nothing", () => {
    expect(NAMES.length).toBeGreaterThanOrEqual(2);
    expect(role(FIRST)).toBeTruthy();
    expect(role(SECOND)).toBeTruthy();
  });

  it("a bare prompt has no persona", () => {
    expect(activePersona(TASK)).toBeNull();
    expect(promptBody(TASK)).toBe(TASK);
  });

  it("applying one prepends the role and is then detectable", () => {
    const withRole = withPersona(TASK, FIRST);
    expect(withRole).toBe(`${role(FIRST)}\n\n${TASK}`);
    expect(activePersona(withRole)).toBe(FIRST);
    expect(promptBody(withRole)).toBe(TASK);
  });

  /** THE ONE THAT MATTERS. */
  it("re-picking SWAPS the role rather than stacking a second one", () => {
    const once = withPersona(TASK, FIRST);
    const twice = withPersona(once, SECOND);

    expect(activePersona(twice)).toBe(SECOND);
    expect(promptBody(twice)).toBe(TASK);
    // The displaced role is GONE, not buried further down the prompt.
    expect(twice).not.toContain(role(FIRST));
    expect(twice).toBe(`${role(SECOND)}\n\n${TASK}`);
  });

  it("clearing removes the role and leaves the author's text", () => {
    const withRole = withPersona(TASK, FIRST);
    const cleared = withPersona(withRole, null);
    expect(cleared).toBe(TASK);
    expect(activePersona(cleared)).toBeNull();
  });

  it("applying the same persona twice is idempotent", () => {
    const once = withPersona(TASK, FIRST);
    expect(withPersona(once, FIRST)).toBe(once);
  });

  it("a persona with no task body is the role alone, and still detectable", () => {
    const roleOnly = withPersona("", FIRST);
    expect(roleOnly).toBe(role(FIRST));
    expect(activePersona(roleOnly)).toBe(FIRST);
    expect(promptBody(roleOnly)).toBe("");
    // …and it can still be swapped out of, which the prompt===role branch handles.
    expect(withPersona(roleOnly, SECOND)).toBe(role(SECOND));
  });

  /**
   * Longest-first matching. If one role's text were a prefix of another's, a naive
   * scan would report the shorter one and then strip the wrong number of characters,
   * corrupting the body. Asserted over the real catalog so it stays true as the
   * catalog grows.
   */
  it("resolves the correct persona even when one role prefixes another", () => {
    for (const name of NAMES) {
      const applied = withPersona(TASK, name);
      expect(activePersona(applied), `${name} resolves to itself`).toBe(name);
      expect(promptBody(applied), `${name} leaves the body intact`).toBe(TASK);
    }
  });

  it("prompt text that merely MENTIONS a role is not treated as that persona", () => {
    const mentions = `Please act like this: ${role(FIRST)}`;
    expect(activePersona(mentions)).toBeNull();
  });
});
