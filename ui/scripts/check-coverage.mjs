#!/usr/bin/env node
/**
 * The console coverage ratchet (zero-dep). Reads the `json-summary` vitest writes and
 * compares it against the committed baseline in `coverage-baseline.json`, failing when any
 * metric has DROPPED. Same posture as the eval harness's `compare_to_baseline`: fail closed
 * on a regression, and never quietly accept a lower number.
 *
 * WHY A RATCHET AND NOT A TARGET. A fixed target ("80% or fail") is a number someone picks
 * once and then argues with. A ratchet asks only that the next change not make things worse,
 * which is the property that actually holds a line — and it makes the current number visible
 * instead of unknown. It was unknown here: the console has 123 spec files and 967 tests, and
 * nothing measured what they reached.
 *
 * TOLERANCE. Percentages move a hundredth of a point on an unrelated refactor, so a
 * zero-tolerance percentage gate fails for reasons no one can act on. The default allows a
 * 0.25-point wobble and no more. That is small enough that removing a test's subject, or
 * adding an untested module of any size, still trips it.
 *
 * RAISING THE BAR IS DELIBERATE. When coverage improves, this prints the new numbers and
 * asks you to update the baseline — it does not update it for you. An auto-updating baseline
 * is not a ratchet; it is a record of whatever last happened.
 *
 *   npm run coverage        # measure (writes coverage/coverage-summary.json)
 *   npm run coverage:check  # gate against the baseline
 *   KX_UI_COVERAGE_UPDATE=1 npm run coverage:check   # rewrite the baseline, reviewed
 */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const UI_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SUMMARY = join(UI_ROOT, "coverage", "coverage-summary.json");
const BASELINE = join(UI_ROOT, "coverage-baseline.json");
const METRICS = ["lines", "statements", "functions", "branches"];
const TOLERANCE_PP = Number(process.env.KX_UI_COVERAGE_TOLERANCE_PP ?? 0.25);

function main() {
  if (!existsSync(SUMMARY)) {
    console.error(
      "check-coverage: no coverage/coverage-summary.json — run `npm run coverage` first.",
    );
    process.exit(1);
  }
  const total = JSON.parse(readFileSync(SUMMARY, "utf8")).total;
  const measured = Object.fromEntries(
    METRICS.map((m) => [m, { pct: total[m].pct, covered: total[m].covered, total: total[m].total }]),
  );

  if (process.env.KX_UI_COVERAGE_UPDATE === "1" || !existsSync(BASELINE)) {
    writeFileSync(BASELINE, `${JSON.stringify({ metrics: measured }, null, 2)}\n`);
    console.log(`wrote ${BASELINE}:`);
    for (const m of METRICS) {
      console.log(`  ${m.padEnd(11)} ${measured[m].pct.toFixed(2)}%`);
    }
    return;
  }

  const baseline = JSON.parse(readFileSync(BASELINE, "utf8")).metrics;
  const regressions = [];
  const gains = [];
  console.log(`console coverage (tolerance ${TOLERANCE_PP} pp):`);
  for (const m of METRICS) {
    const now = measured[m].pct;
    const was = baseline[m]?.pct;
    if (was === undefined) {
      regressions.push(`${m}: absent from the baseline — regenerate it`);
      continue;
    }
    const delta = now - was;
    const mark = delta < -TOLERANCE_PP ? "✗" : "✓";
    console.log(
      `  ${mark} ${m.padEnd(11)} ${now.toFixed(2)}%  (baseline ${was.toFixed(2)}%, ` +
        `${delta >= 0 ? "+" : ""}${delta.toFixed(2)} pp)  ` +
        `${measured[m].covered}/${measured[m].total}`,
    );
    if (delta < -TOLERANCE_PP) {
      regressions.push(`${m} fell ${Math.abs(delta).toFixed(2)} pp (${was.toFixed(2)}% → ${now.toFixed(2)}%)`);
    } else if (delta > TOLERANCE_PP) {
      gains.push(`${m} rose ${delta.toFixed(2)} pp`);
    }
  }

  if (regressions.length > 0) {
    console.error("\nFAIL: console coverage regressed:");
    for (const r of regressions) console.error(`  ${r}`);
    console.error("\nCover the code you added, or — if the drop is intended — update the");
    console.error("baseline in the same commit with KX_UI_COVERAGE_UPDATE=1 and say why.");
    process.exit(1);
  }

  if (gains.length > 0) {
    console.log("\nCoverage improved:");
    for (const g of gains) console.log(`  ${g}`);
    console.log("Raise the ratchet in this commit: KX_UI_COVERAGE_UPDATE=1 npm run coverage:check");
  }
  console.log("\nOK: no coverage regression.");
}

main();
