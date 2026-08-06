#!/usr/bin/env node
/**
 * The eager-JS bundle gate (W1, zero-dep). The EAGER set is exactly what the
 * browser loads before any user interaction: the `<script type="module">` entry
 * plus every `<link rel="modulepreload">` chunk Vite emits into `dist/index.html`
 * (statically-imported vendor chunks). Lazy chunks (MoteDag, sections, the
 * motion-features pack, the DevTools dock) are reported but NOT counted.
 *
 * Budget: 688 KiB raw (704,512 B — the value enforced below; keep this line in
 * lock-step with it, and with the step name in ci.yml, or the doc becomes the
 * third place that disagrees). Override with KX_UI_EAGER_BUDGET_BYTES for
 * emergencies — a deliberate, reviewed override, never a silent default bump.
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
 *   - 657 KiB → 659 KiB (Apps closeout): the App lifecycle gains a DELETE — DeleteApp's
 *     request/response messages, the RPC stub, and `client.deleteApp` — plus the hosted
 *     serve lane's additive wire surface (HostedAppState::HOSTED_BUILDING,
 *     HostedAppStatus.serve_mode) and its SDK mapping. All of it rides the eager
 *     `common.js`: connection-context constructs the client up front, so a generated
 *     message schema cannot be lazy-split per feature. Measured 671,761 B (origin/main)
 *     → 673,979 B (+2,218 B eager); bumped to the next KiB boundary.
 *   - 659 KiB → 660 KiB (run-view scoping): `RunHandle` gains `terminal_mote_id` — the
 *     general run anchor, populated for every shape where `react_chain_salt` covers only
 *     a single tool-granted agentic step — so the generated message schema grows again on
 *     the eager `common.js`. Alongside it `lib/run-anchor` (the anchor rule, written once
 *     instead of repeated at a dozen navigations) is pulled eager by the route modules
 *     that validate `?chain=`/`?anchor=`. Measured 673,979 B (origin/main) → 674,866 B
 *     (+887 B eager); bumped to the next KiB boundary.
 *   - 665 KiB → 670 KiB (the script primitive): the eager SDK client gains
 *     registerScript / listScripts / getScript / deregisterScript, the `RegisteredScript`
 *     model, and the regenerated proto descriptor for the four new messages. Like every
 *     entry above it rides the eager `common.js`, which connection-context loads up front,
 *     so a generated message schema cannot be lazy-split per feature. The two new PANELS
 *     are route-lazy and cost nothing eager.
 *
 *     Measured with clean installs of BOTH packages, A/B against origin/main with the SDK
 *     rebuilt from main too (rebuilding only the UI leaves this branch's SDK `dist` in
 *     place — it is gitignored, so a stash does not revert it, and the "baseline" then
 *     silently includes the change under test): main 683,190 B → this branch 685,034 B
 *     (+1,844 B eager). Bumped to the next KiB boundary above the measured value.
 *
 *     NOTE for whoever reads this next: those local numbers put ORIGIN/MAIN itself over the
 *     previous 680,960 B budget, yet main's CI is green — so the local toolchain measures a
 *     few KiB heavier than CI's (both are node 22; CI resolves a later patch). The DELTA is
 *     the trustworthy half of the measurement; the absolute is not comparable across hosts.
 *   - 670 KiB → 673 KiB (branch point-in-time history + the create route): the eager SDK
 *     client gains listBranchVersions / restoreBranch, the `BranchVersion` /
 *     `RestoreResult` models, the regenerated descriptor for the four new messages, and
 *     `AppSummary.lifecycle` (the draft badge field, carried on the summary so ONE
 *     listApps paints every badge) — all riding the eager `common.js` like every entry
 *     above. The `/apps/create` route REGISTRATION (createRoute + validateSearch; the
 *     screen itself is lazy) adds ~700 B via router.tsx. Measured A/B with the SDK
 *     rebuilt from main for the baseline (same protocol as the script-primitive entry):
 *     main 684,952 B → SDK growth 687,338 B → + the route module 688,063 B
 *     (+3,111 B eager total); bumped to the next KiB boundary above the measured value.
 *   - 673 KiB → 678 KiB (2026-07-30, durable Workflows): the eager SDK client
 *     gains saveWorkflow / listWorkflows / getWorkflow / runWorkflow / deleteWorkflow +
 *     the regenerated proto descriptor for the ten new messages (incl. RunWorkflow's
 *     RunHandle parity and `RegisterTriggerRequest.workflow_handle`) — riding the eager
 *     `common.js` like every entry above (connection-context constructs the client up
 *     front). The two new route REGISTRATIONS (`/workflows/create` + `/workflows/def/
 *     $handle`; both screens lazy) add the usual ~600 B each via router.tsx. Measured
 *     A/B on the feature branch with the SDK dist held constant: branch UI without the
 *     pre-UI surface 692,971 B (the SDK tranche's growth, its bump deferred to this UI
 *     tranche) → with it 694,167 B (+1,196 B for the routes; +5,015 B total over the
 *     old ceiling); bumped to the next KiB boundary above the measured value. Local
 *     toolchain measures a few KiB heavier than CI (see the NOTE above) — the delta is
 *     the trustworthy half.
 *   - 678 KiB → 685 KiB (2026-07-31, the NL authoring surface): the eager SDK
 *     client gains proposeControlAction / describeControlSurface / putPolicyRole /
 *     listPolicyRoles / deletePolicyRole / assignPolicyRole plus the regenerated
 *     proto descriptor for their messages — including `ControlPreview`, a oneof
 *     over eight request types, which is the largest single message added. Proto
 *     schemas cannot be lazy-split and connection-context constructs the client
 *     up front, so this rides eager `common.js` like every entry above. Plus the
 *     `use-live-invalidation` hook mounted at AppShell (no new route, no new
 *     screen — it is a subscription, not a page).
 *     Measured A/B, both arms built by CI's OWN procedure in the same session —
 *     `npm ci && npm run build` in bindings/typescript FIRST, then a clean
 *     `npm ci && npm run build` in ui — because the UI takes the SDK's built dist
 *     via a file dependency and a stale dist silently changes the number:
 *       origin/main 694,167 B → this branch 701,397 B  (+7,230 B eager, 10‰)
 *     The three vendor chunks are BYTE-IDENTICAL across both arms (same sizes AND
 *     the same content hashes: vendor-react-CCeFEH2b / vendor-router-kxFVRzc4 /
 *     vendor-query-D83rVUso), so the entire delta is in the entry chunk —
 *     419,532 B → 426,762 B — which is what makes it a real cost rather than a
 *     toolchain artefact. Bumped to the next KiB boundary above the measured
 *     value: 685 KiB = 701,440 B, 43 B of headroom.
 *
 *     THE FIRST ATTEMPT AT THIS ENTRY WAS WRONG, and how it was wrong is the
 *     reason the procedure above is spelled out. It recorded 695,680 B and bumped
 *     to 680 KiB — a number measured against a STALE SDK dist, compared against a
 *     main figure QUOTED from the entry above rather than rebuilt. Neither arm was
 *     built the same way, so the "+1,513 B" was not a delta at all. CI measured
 *     701,397 B and failed the gate. Rebuilt honestly, main reproduces 694,167 B
 *     exactly — the number the previous entry recorded — and the branch reproduces
 *     CI's 701,397 B exactly. An A/B is only an A/B when both arms are built by
 *     the same procedure; quoting one side from history is not measuring it.
 *
 *     Note also that 680 KiB was never viable: main ALONE was 694,167 B against a
 *     696,320 B ceiling — 2,153 B of headroom before this branch added anything.
 *
 *   685 KiB -> 688 KiB, the run view reads a run's agentic chain. The projection
 *     hook gains the chain fetch + the roster builder, and `dag-graph` gains the
 *     multi-anchor component walk; both modules are already eager because the
 *     route tree imports the run route statically. The DRAWING of the chain —
 *     `derived-lineage.ts` — is in the lazy MoteDag chunk and costs zero here,
 *     which is why the delta is under 1 KiB rather than the several the visual
 *     might suggest. The roster builder deliberately RESTATES the turn-ordering
 *     rule instead of importing it from `use-react-progress`: that module has no
 *     eager importer today, and importing it would hoist a whole lazy chunk into
 *     the entry.
 *     Measured A/B by the procedure above, both arms in one session against the
 *     SAME freshly-built SDK dist (this PR changes no SDK surface, so the SDK is
 *     constant across arms — the cleanest A/B in this file's history):
 *       baseline 700,072 B -> this branch 701,719 B  (+1,647 B eager, ~2‰)
 *     ⚠ The branch FITS the old 701,440 B ceiling on this host and still needs the
 *     bump, because the ABSOLUTE is not comparable across hosts and the DELTA is:
 *     CI measured 701,397 B for the same baseline (1,325 B above this host), so
 *     CI lands at ~703,044 B and would fail. Bumped to 688 KiB = 704,512 B, leaving
 *     ~1,468 B of projected CI headroom — one KiB above the next boundary, because
 *     the host offset is an ESTIMATE and a 444 B margin is not one.
 *
 * Exit 1 over budget. The printed table doubles as the measurement evidence blob.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const UI_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DIST = join(UI_ROOT, "dist");
// 662 KiB -> 665 KiB for per-node capability bindings. The growth is the TypeScript SDK's,
// not the console's, and it was measured three ways rather than assumed: main UI + main SDK
// = 677_339 B; THIS branch's UI + main SDK = 677_329 B (10 B SMALLER — the per-node drawer,
// the stripped create form and the rails are all route-lazy, so the console's eager cost is
// flat); this branch's UI + this branch's SDK = 679_964 B. So all +2_635 B is the regenerated
// proto descriptor for the three new `DerivedAppStep` fields plus the `DagSpecStep` /
// `deriveApp` mapping additions, and the SDK client is eager on every route. Adding a
// per-step contract axis buys that; bumping here — in the PR that spends it — is the same move
// #375, #363, #362, #358 and #304 each made.
const BUDGET = Number(process.env.KX_UI_EAGER_BUDGET_BYTES ?? 704_512);

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
