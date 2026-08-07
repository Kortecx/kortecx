import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("Tools registry: built-in inventory, disabled built-in deregister, SSRF-refused host, live Connections panel", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-tools").click();
  await expect(page.getByTestId("tools-section")).toBeVisible();

  // The durable registry inventory (DiscoverTools) shows the OSS built-ins, re-seeded
  // on open (DISTINCT from the advisory toolscout manifests below). `text-summarize@1`
  // was removed from the built-in set — it had no implementation anywhere, so it
  // advertised a tool that could never dispatch.
  await expect(page.getByTestId("tools-registered-panel")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("registered-tool-fs-read-1")).toBeVisible();
  await expect(page.getByTestId("registered-tool-fs-write-1")).toBeVisible();
  await expect(page.getByTestId("registered-tool-text-summarize-1")).toHaveCount(0);

  // Built-ins are re-seeded on start and NOT deregisterable — the control is disabled.
  await expect(page.getByTestId("deregister-fs-read-1")).toBeDisabled();

  // Register an internal/loopback host → SSRF admission refuses it (permission_denied
  // → "Host not permitted"). The inputs are CONTROLLED — click + pressSequentially,
  // never a bulk fill() (the recorded React-controlled-input e2e gotcha).
  const name = page.getByTestId("register-tool-name");
  await name.click();
  await name.pressSequentially("web-search");
  const host = page.getByTestId("register-tool-host");
  await host.click();
  await host.pressSequentially("127.0.0.1:443");
  await page.getByTestId("register-tool-submit").click();
  await expect(page.getByTestId("register-tool-error")).toContainText("Host not permitted", {
    timeout: 30_000,
  });

  // The Connectors surface — the govern surface over the external MCP gateway, merged
  // with the bundled-connector catalog it used to sit beside. The configure form is
  // opened FROM a row rather than always present, so the flow is: the list renders, a
  // row offers set-up, and taking it opens the form. The honest-disabled Cloud
  // affordance is always there, regardless of whether this FFI-free serve wired the
  // mcp-gateway feature.
  await page.getByTestId("tools-tab-connectors").click();
  await expect(page.getByTestId("connectors-panel")).toBeVisible();
  await expect(page.getByTestId("connectors-cloud-disabled")).toBeVisible();
  await page.getByTestId("connector-configure-gmail").click();
  await expect(page.getByTestId("connector-form")).toBeVisible();
  // Prefilled from the row, so setting up a bundled connector is not retyping it.
  await expect(page.getByTestId("connector-name")).toHaveValue("gmail");
});
