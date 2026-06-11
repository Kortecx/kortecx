/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// One config drives the dev server, the production build, AND vitest. The dev
// (5173) / preview (4173) ports are pinned + strict so the gateway's deny-by-default
// `--cors-origin` allowlist can name them exactly (a flaky CORS mismatch otherwise).
// The EAGER vendor split (cache-stable, machine-checkable via modulepreload links
// in dist/index.html — the eager set `scripts/check-bundle-size.mjs` gates ≤600KB).
// An explicit ALLOWLIST, never a catch-all `node_modules → vendor`: a catch-all
// would hoist @xyflow/dagre (which must stay inside the lazy MoteDag chunk) and
// framer-motion (whose animation engine must stay in the dynamic motion-features
// chunk — the LazyMotion split) into the eager set.
function vendorChunk(id: string): string | undefined {
  if (!id.includes("node_modules")) {
    return undefined;
  }
  if (/node_modules\/(react|react-dom|scheduler)\//.test(id)) {
    return "vendor-react";
  }
  if (/node_modules\/@tanstack\/(react-router|router-core|history|react-store|store)\//.test(id)) {
    return "vendor-router";
  }
  if (/node_modules\/@tanstack\/(react-query|query-core)\//.test(id)) {
    return "vendor-query";
  }
  return undefined;
}

export default defineConfig({
  plugins: [react()],
  server: { port: 5173, strictPort: true },
  preview: { port: 4173, strictPort: true },
  build: {
    rollupOptions: {
      output: { manualChunks: vendorChunk },
    },
  },
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
