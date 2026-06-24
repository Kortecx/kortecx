/**
 * D139 — THE REAL INSTALLED FLOW: `kx serve` itself hosts the console (no vite
 * dev/preview server anywhere). The browser loads the SPA from kx's embedded-
 * console listener, connects to the gateway's gRPC-web port (whose CORS
 * allowlist auto-granted the console's own origin — zero --cors-origin flags),
 * runs a blueprint, and watches it commit.
 *
 * Gated on KX_CONSOLE_E2E=1: the spec needs a `--features console` kx (the CI
 * ui job builds one; locally run `just console-build` and set KX_BIN).
 */

import { expect, test } from "@playwright/test";
import { gotoViaPalette } from "./fixtures/connect";
import { type Gateway, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test.skip(
  process.env.KX_CONSOLE_E2E !== "1",
  "needs a --features console kx (set KX_CONSOLE_E2E=1 with KX_BIN pointing at one)",
);

test("the embedded console serves the SPA and runs a blueprint end to end", async ({ page }) => {
  gw = await spawnGateway({ console: true });
  const console_ = gw.consoleOrigin;
  expect(console_).toBeTruthy();

  // The SPA loads FROM kx itself — the login gate renders.
  await page.goto(`${console_}/connect`);
  await expect(page.getByTestId("app-gate")).toBeVisible();
  await expect(page.getByRole("heading", { name: /connect to a gateway/i })).toBeVisible();

  // Connect to the gateway endpoint; the CORS self-grant makes this work with
  // ZERO --cors-origin flags (the proof of the D139.3 auto-extension).
  const endpoint = page.getByLabel(/gateway endpoint/i);
  await endpoint.fill(gw.endpoint);
  await page.getByRole("button", { name: /^connect$/i }).click();
  await expect(page.getByTestId("chat-panel")).toBeVisible({ timeout: 30_000 });

  // Run the echo blueprint through the embedded console and watch it commit.
  await gotoViaPalette(page, "recipes");
  await expect(page.getByTestId("recipe-catalog")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("recipe-pick-kx/recipes/echo").click();
  const topic = page.getByTestId("field-topic");
  await topic.click();
  await topic.pressSequentially("embedded console");
  await page.getByRole("button", { name: /run blueprint/i }).click();
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).first()).toBeVisible(
    { timeout: 30_000 },
  );

  // SPA fallback: a deep link served by kx still lands in the app.
  await page.goto(`${console_}/settings`);
  await expect(page.getByTestId("app-gate")).toBeVisible(); // reload drops the token → gate
});
