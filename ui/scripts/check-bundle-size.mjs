#!/usr/bin/env node
/**
 * The eager-JS bundle gate (W1, zero-dep). The EAGER set is exactly what the
 * browser loads before any user interaction: the `<script type="module">` entry
 * plus every `<link rel="modulepreload">` chunk Vite emits into `dist/index.html`
 * (statically-imported vendor chunks). Lazy chunks (MoteDag, sections, the
 * motion-features pack, the DevTools dock) are reported but NOT counted.
 *
 * Budget: 656 KiB raw (override with KX_UI_EAGER_BUDGET_BYTES for emergencies —
 * a deliberate, reviewed override, never a silent default bump).
 *
 * History (deliberate, reviewed default bumps — each tied to a real capability the
 * eager SDK client must carry; the SDK is loaded by connection-context up front, so
 * an eager-surface addition can't be lazy-split per-feature):
 *   - 600 KiB → 624 KiB (D170 Integrations Foundation): +13 proto messages / +2 enums
 *     for the secrets (PutSecret/ListSecretNames/DeleteSecret) + triggers
 *     (Register/List/Deregister/Submit/TestTrigger) RPC surface, plus the
 *     `client.secrets`/`client.triggers` methods + result types (~6 KiB eager).
 *   - 624 KiB → 640 KiB (RC5b durable-memory decay/consolidation): +6 proto messages
 *     (DecayMemory/MemoryStats/RestoreMemory req+resp) + MemorySummary salience/
 *     tombstone fields, plus the `client.memory.{decay,stats,restore,consolidate}`
 *     methods + DecayReport/MemoryStats/DecayCandidate result types (~2 KiB eager).
 *   - 640 KiB → 648 KiB (multi-agent orchestration layer): the eager Flow client gains the
 *     supervisor() / consensus() / reviewLoop() orchestration methods + their default
 *     planner/gather/judge/review prompt constants + the consensus-vote key. Pure client
 *     composition (no new proto / RPC), but it rides the eager `common.js`. Measured
 *     654,787 B (origin/main) → 656,790 B (~2 KiB eager); bumped to the next KiB boundary.
 *   - 648 KiB → 656 KiB (portable App bundles): the eager SDK client gains
 *     exportAppBundle() / importApp() / cloneApp() + the `source_digest` field on
 *     SaveApp/GetApp + the `kortecx.appbundle/v1` codec (base64 + envelope walk). Rides
 *     the eager `common.js` (loaded up front by connection-context). Measured 656,790 B
 *     (origin/main) → 662,868 B (~6 KiB eager); bumped to the next KiB boundary.
 *   - 656 KiB → 657 KiB (POC-6 live agentic creation): +3 additive fields on
 *     GetScaffoldStatusResponse (writing_path/writing_instance_id/writing_mote_id) so the
 *     scaffold surfaces the live-writing file's token-stream ids — the generated message
 *     schema is eager (connection-context loads the client up front). Measured 671,744 B
 *     (origin/main) → 671,761 B (+17 B eager); bumped to the next KiB boundary.
 *
 * Exit 1 over budget. The printed table doubles as the GR10 evidence blob.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const UI_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DIST = join(UI_ROOT, "dist");
const BUDGET = Number(process.env.KX_UI_EAGER_BUDGET_BYTES ?? 672_768);

/** Pull the eager JS URLs out of dist/index.html (entry scripts + modulepreloads). */
export function eagerJsUrls(html) {
  const urls = new Set();
  for (const m of html.matchAll(/<script[^>]+type="module"[^>]*\ssrc="([^"]+\.js)"/g)) {
    urls.add(m[1]);
  }
  for (const m of html.matchAll(/<link[^>]+rel="modulepreload"[^>]*\shref="([^"]+\.js)"/g)) {
    urls.add(m[1]);
  }
  return [...urls];
}

function main() {
  const html = readFileSync(join(DIST, "index.html"), "utf8");
  const eager = eagerJsUrls(html);
  if (eager.length === 0) {
    console.error("check-bundle-size: no eager JS found in dist/index.html — did the build run?");
    process.exit(1);
  }

  let total = 0;
  const rows = [];
  for (const url of eager) {
    const path = join(DIST, url.replace(/^\//, ""));
    const bytes = statSync(path).size;
    total += bytes;
    rows.push([url, bytes]);
  }
  rows.sort((a, b) => b[1] - a[1]);

  console.log("eager JS (entry + modulepreload):");
  for (const [url, bytes] of rows) {
    console.log(`  ${String(bytes).padStart(9)} B  ${url}`);
  }
  console.log(`  ${String(total).padStart(9)} B  TOTAL (budget ${BUDGET} B)`);

  // Informational: the lazy remainder (everything else under dist/assets).
  const eagerNames = new Set(rows.map(([u]) => u.split("/").pop()));
  let lazyTotal = 0;
  for (const f of readdirSync(join(DIST, "assets"))) {
    if (f.endsWith(".js") && !eagerNames.has(f)) {
      lazyTotal += statSync(join(DIST, "assets", f)).size;
    }
  }
  console.log(`  ${String(lazyTotal).padStart(9)} B  lazy remainder (not gated)`);

  if (total > BUDGET) {
    console.error(`\nFAIL: eager JS ${total} B exceeds the ${BUDGET} B budget.`);
    process.exit(1);
  }
  console.log("\nOK: eager JS within budget.");
}

// Import-safe for the parser unit test; executes when run directly.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
