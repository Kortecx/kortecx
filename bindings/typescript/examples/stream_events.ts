/**
 * Stream a run's events with a native async iterator.
 *
 * Start a run WITHOUT waiting (get a `Run` handle), then consume its event deltas
 * as they land. `follow: false` reads one snapshot and stops; `follow: true` keeps
 * the live tail open (resuming transparently on a catch-up drop).
 *
 *   npx tsx examples/stream_events.ts
 */

import { KxClient, type Run } from "@kortecx/sdk";

async function main(): Promise<void> {
  const kx = new KxClient("http://127.0.0.1:50151");
  try {
    const run = (await kx.invoke("kx/recipes/echo", { topic: "watch me" })) as Run;
    console.log("instance_id:", run.instanceId);
    for await (const delta of run.events({ since: 0n, follow: false })) {
      console.log(`seq=${delta.seq} ${delta.kind} ${delta.moteId ?? delta.targetMoteId ?? ""}`);
    }
  } finally {
    kx.close();
  }
}

main().catch((err) => {
  console.error(String(err));
  process.exitCode = 1;
});
