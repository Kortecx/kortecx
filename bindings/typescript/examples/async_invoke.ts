/**
 * Low-latency wait via the live event stream (`waitMode: "events"`).
 *
 * Identical result to the default poll wait, but reacts to the terminal Mote's
 * committed delta as it lands (sub-second) instead of polling the projection.
 *
 *   npx tsx examples/async_invoke.ts
 */

import { KxClient, type Result } from "@kortecx/sdk";

async function main(): Promise<void> {
  const kx = new KxClient("http://127.0.0.1:50151");
  try {
    const result = (await kx.invoke(
      "kx/recipes/echo",
      { topic: "low latency" },
      { wait: true, waitMode: "events" },
    )) as Result;
    console.log(result.ok ? "✓ committed" : `✗ ${result.state}`, result.instanceId);
    console.log("result:", result.text);
  } finally {
    kx.close();
  }
}

main().catch((err) => {
  console.error(String(err));
  process.exitCode = 1;
});
