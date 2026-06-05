/**
 * invoke + wait — the "runtime as a function" path.
 *
 * Run a published recipe and block for its committed result. Start a gateway
 * first: `kx serve --dev-allow-local --journal /tmp/kx.db --content /tmp/kx-blobs`.
 *
 *   npx tsx examples/invoke_wait.ts
 */

import { KxClient, type Result } from "@kortecx/sdk";

async function main(): Promise<void> {
  const kx = new KxClient("http://127.0.0.1:50151");
  try {
    const result = (await kx.invoke(
      "kx/recipes/echo",
      { topic: "hello" },
      { wait: true },
    )) as Result;
    console.log("state           :", result.state);
    console.log("instance_id     :", result.instanceId);
    console.log("terminal_mote_id:", result.terminalMoteId);
    console.log("result          :", result.text);
  } finally {
    kx.close();
  }
}

main().catch((err) => {
  console.error(String(err));
  process.exitCode = 1;
});
