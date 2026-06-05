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
 */

export { KxClient } from "./node.js";
export * from "./common.js";
