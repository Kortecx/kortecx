import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

/**
 * The MCP hub (D170 foundation seam) — the Tools section is a tabbed hub:
 * Tools | Scripts | Integrations | Connections | Skills | Triggers | Secrets. This
 * is a RENDER-level check (the Triggers/Secrets stores may degrade to a not-wired
 * state on an FFI-free serve, so we assert the tab switch + the always-present
 * register/add forms, never a live trigger fire — keeping it flake-free per the
 * e2e harness guidance).
 */
test("MCP hub: Scripts, Triggers + Secrets tabs render their govern forms", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-tools").click();
  await expect(page.getByTestId("tools-section")).toBeVisible();
  // The relabelled section reads "MCP" with its tab toggle.
  // `exact` + the level matter here: accessible-name matching is a substring by
  // default, and "Register an external MCP tool" further down the page also
  // contains "MCP", so an unqualified match is ambiguous rather than absent.
  await expect(page.getByRole("heading", { level: 1, name: "MCP", exact: true })).toBeVisible();
  await expect(page.getByTestId("tools-tabs")).toBeVisible();

  // Scripts tab → the registry view and its register form. A serve without the
  // script RPCs degrades to an honest not-wired empty state, so assert whichever
  // of the two rendered rather than pinning the wired case and flaking on a build
  // that does not have it.
  await page.getByTestId("tools-tab-scripts").click();
  await expect(
    page.getByTestId("scripts-panel").or(page.getByText("Scripts need a newer gateway")),
  ).toBeVisible();

  // Integrations tab → the bundled connectors, which are a static catalog and so
  // always render regardless of what the gateway reports.
  await page.getByTestId("tools-tab-integrations").click();
  await expect(page.getByTestId("integrations-panel")).toBeVisible();
  await expect(page.getByTestId("integration-dial-gmail")).toBeVisible();

  // Triggers tab → the register form is always present (kind + auth CHIP groups,
  // recipe handle, submit), independent of whether the registry store is wired.
  await page.getByTestId("tools-tab-triggers").click();
  await expect(page.getByTestId("triggers-panel")).toBeVisible();
  await expect(page.getByTestId("trigger-add-form")).toBeVisible();
  await expect(page.getByTestId("trigger-kind-webhook")).toBeVisible();
  await expect(page.getByTestId("trigger-kind-cron")).toBeVisible();
  await expect(page.getByTestId("trigger-auth-none")).toBeVisible();

  // Switching the kind to cron via the CHIP control reveals the schedule field
  // (controlled selects are avoided — the recorded React-controlled-select gotcha).
  await page.getByTestId("trigger-kind-cron").click();
  await expect(page.getByTestId("trigger-kind-cron")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("trigger-add-schedule")).toBeVisible();
  // A non-none auth reveals the secret-ref field.
  await page.getByTestId("trigger-auth-hmac_sha256").click();
  await expect(page.getByTestId("trigger-add-secret-ref")).toBeVisible();

  // Secrets tab → the add form with a write-only (type=password) value field.
  await page.getByTestId("tools-tab-secrets").click();
  await expect(page.getByTestId("secrets-panel")).toBeVisible();
  await expect(page.getByTestId("secret-add-form")).toBeVisible();
  await expect(page.getByTestId("secret-add-value")).toHaveAttribute("type", "password");
});
