/**
 * THE MOTION LAYER, and the only spec in the suite that can see it.
 *
 * ⚠ WHY THIS FILE EXISTS AT ALL. `e2e/fixtures/connect.ts` calls
 * `page.emulateMedia({ reducedMotion: "reduce" })` on every run, deliberately and with
 * a recorded reason (the shell skips its route transition under reduced motion; an
 * rAF-driven exit animation can remount a route mid-test — a ~5-in-6 flake). That
 * fixture must NOT be removed. The consequence is that every animation in the console
 * is silenced for the entire default suite, so motion work is untested unless one spec
 * opts back in. This is that spec.
 *
 * Two independent claims, because neither alone settles it:
 *
 *  1. **Motion is STATE-DRIVEN.** A settled graph carries no live edges at all. This is
 *     the claim that matters for correctness — an animation that keeps running on a
 *     finished run is not motion, it is noise, and it would make the console lie about
 *     whether anything is still happening.
 *  2. **The reduced-motion guard actually bites.** The same element, in the same page,
 *     reports a running animation under `no-preference` and `none` under `reduce`. An
 *     A/B on one variable — without the `reduce` arm, claim 1 would pass just as well
 *     against CSS that never animates anything, and the accessibility bound would be
 *     asserted by a comment.
 *
 * Model-free (the `passthrough-dag` recipe through a real gRPC-web gateway), so it runs
 * in the default suite rather than behind a live-model env gate.
 */

import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

/** Read the resolved animation on a probe carrying `cls`, under the current media
 *  emulation. A probe rather than a hunted-for live edge: whether a model-free run is
 *  ever caught mid-flight is a race, and the CSS contract is what is under test. */
async function probeAnimation(page: import("@playwright/test").Page, cls: string) {
  return page.evaluate((className) => {
    const el = document.createElement("div");
    el.className = className;
    document.body.appendChild(el);
    const s = getComputedStyle(el);
    const out = { name: s.animationName, duration: s.animationDuration };
    el.remove();
    return out;
  }, cls);
}

test("the run graph's motion is state-driven and honours reduced motion", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // ── OPT BACK IN. `connectConsole` set `reduce`; everything below the first arm
  // depends on this line, and it is the whole reason the file exists.
  await page.emulateMedia({ reducedMotion: "no-preference" });

  await runRecipe(page, { handle: "kx/recipes/passthrough-dag" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.getByTestId("mote-node").count(), { timeout: 30_000 }).toBe(5);

  // ── CLAIM 2a: under no-preference the live-edge rule animates. Asserted BEFORE the
  // settled-state claim, so a failure here is not mistaken for "the run finished".
  const moving = await probeAnimation(page, "dag-edge dag-edge--data dag-edge--live");
  expect(
    moving.name,
    "the live-edge rule resolves to no animation under `no-preference` — the motion " +
      "layer is not reaching the page at all, and every other assertion here would be vacuous",
  ).not.toBe("none");
  expect(moving.duration).not.toBe("0s");

  // ── CLAIM 2b: THE CONTROL. Same element, same page, one variable changed.
  await page.emulateMedia({ reducedMotion: "reduce" });
  const still = await probeAnimation(page, "dag-edge dag-edge--data dag-edge--live");
  expect(
    still.name,
    "the live-edge animation still resolves under `prefers-reduced-motion: reduce` — " +
      "the accessibility bound the motion layer is explicitly held to is not being enforced",
  ).toBe("none");
  await page.emulateMedia({ reducedMotion: "no-preference" });

  // ── CLAIM 1: a SETTLED graph is completely still. Every Mote commits, and when it
  // has, no edge may still be marked live.
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBe(5);
  await expect.poll(() => page.locator(".dag-edge--live").count(), { timeout: 15_000 }).toBe(0);

  // …and the precondition that makes that zero MEAN something: there are edges to have
  // been live. A graph with no edges at all would satisfy the count above trivially.
  expect(
    await page.locator(".dag-edge").count(),
    "no edges on the canvas — the zero-live-edges assertion above is vacuous",
  ).toBeGreaterThan(0);
});
