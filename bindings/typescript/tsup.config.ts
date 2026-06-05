import { defineConfig } from "tsup";

// Dual ESM + CJS build with type declarations. Three entrypoints: the root (`.`,
// Node default), `./node` (explicit Node gRPC transport), and `./web` (browser
// gRPC-web transport). Runtime deps are externalized — the bundle is just our
// transpiled source + the vendored generated stubs.
export default defineConfig({
  entry: {
    index: "src/index.ts",
    node: "src/node.ts",
    web: "src/web.ts",
  },
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  target: "es2022",
  splitting: false,
});
