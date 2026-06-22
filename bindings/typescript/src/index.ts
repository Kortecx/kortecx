/**
 * kortecx — TypeScript/JavaScript client SDK for the durable agentic-execution
 * runtime. A pure gRPC + gRPC-web client over the frozen `KxGateway` contract
 * (NOT a native FFI binding). The root export is the Node client; browser/
 * dashboard code imports from `@kortecx/sdk/web`.
 *
 * ```ts
 * import { KxClient } from "@kortecx/sdk";
 *
 * const kx = new KxClient("http://127.0.0.1:50151");
 * const result = await kx.invoke("kx/recipes/echo", { topic: "hello" }, { wait: true });
 * console.log(result.text);
 * ```
 *
 * Or zero-config (V2a) — no constructor, a lazily-built default client from env /
 * `~/.kortecx/config.toml`:
 *
 * ```ts
 * import { run, flow } from "@kortecx/sdk";
 * const out = await run(flow().agent("Summarize the README"));
 * ```
 */

export { KxClient } from "./node.js";
export * from "./common.js";
// V2a g1: the Node zero-config surface (run / invoke / makeClient / defaultClient /
// setDefaultClient) — importing this entry installs the lazy default-client factory the
// Flow/Agent terminals use. NODE-ONLY (the web/chains entries are explicit-client).
export * from "./defaults.js";
// PR-9c-1: the embeddable agent-runner. NODE-ONLY (uses the zero-config default
// client); the `AgentResult`/`AuditedAction` data types ride `common` (web-safe).
export { runAgent } from "./run-agent.js";
export type { RunAgentOptions } from "./run-agent.js";
