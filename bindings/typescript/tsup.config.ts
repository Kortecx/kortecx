import { defineConfig } from "tsup";

// Dual ESM + CJS build with type declarations. Four entrypoints: the root (`.`,
// Node default), `./node` (explicit Node gRPC transport), `./web` (browser
// gRPC-web transport), and `./chains` (the transport-free Chains DSL — the string
// DSL + combinators that lower to the BlueprintBuilder). Runtime deps are
// externalized — the bundle is just our transpiled source + the vendored stubs.
export default defineConfig({
  entry: {
    index: "src/index.ts",
    node: "src/node.ts",
    web: "src/web.ts",
    chains: "src/chains.ts",
  },
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  target: "es2022",
  splitting: false,
});
