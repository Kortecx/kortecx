import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    // The contract tests spawn a real `kx serve` and build the binary on a cold
    // cache, so the budgets are generous (mirrors the Python suite's tolerance).
    testTimeout: 120_000,
    hookTimeout: 180_000,
    // One gateway per file keeps the run-instance-per-recipe rule simple.
    fileParallelism: false,
  },
});
