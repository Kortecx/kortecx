/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// One config drives the dev server, the production build, AND vitest. The dev
// (5173) / preview (4173) ports are pinned + strict so the gateway's deny-by-default
// `--cors-origin` allowlist can name them exactly (a flaky CORS mismatch otherwise).
export default defineConfig({
  plugins: [react()],
  server: { port: 5173, strictPort: true },
  preview: { port: 4173, strictPort: true },
  test: {
    // Component/unit tests run in jsdom; the contract test opts into the node
    // environment per-file (`// @vitest-environment node`) so it gets real
    // `child_process`/`net` + undici fetch against a live `kx serve`.
    environment: "jsdom",
    globals: false,
    setupFiles: ["./test/setup.ts"],
    include: ["test/**/*.test.{ts,tsx}"],
    exclude: ["e2e/**", "node_modules/**", "dist/**"],
    // The contract test builds the FFI-free `kx` on a cold cache + spawns it, so
    // the budgets mirror the SDK suite's tolerance.
    testTimeout: 120_000,
    hookTimeout: 180_000,
    // One gateway per file keeps the run-instance-per-recipe rule simple.
    fileParallelism: false,
  },
});
