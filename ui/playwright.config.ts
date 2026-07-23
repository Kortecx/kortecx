import { defineConfig, devices } from "@playwright/test";

// The SPA is served (production bundle) at a PINNED origin so each test's
// `kx serve` can allow exactly `http://localhost:4174` in its deny-by-default
// `--cors-origin` allowlist. The gateway port is random per test (fixtures), but
// the browser Origin is always the preview origin.
export default defineConfig({
  testDir: "e2e",
  fullyParallel: false,
  workers: 1,
  timeout: 120_000,
  expect: { timeout: 30_000 },
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["html", { open: "never" }], ["list"]] : "list",
  use: {
    baseURL: "http://localhost:4174",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    // Serves the built dist/ — run `npm run build` first (CI + `verify` do).
    command: "npm run preview",
    url: "http://localhost:4174",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
