#!/usr/bin/env node
// docs/site/scripts/render-bench-chart.mjs — emit the README's two-engine capability
// comparison FROM the committed baselines.
//
// WHY A GENERATOR. The published benchmark block used to be hand-maintained, and a
// hand-maintained block ages badly in a way its own gate could not see: `check-docs`
// validated every NUMBER against the baselines while the sentence above them still named a
// capture from two weeks and fourteen commits earlier. The numbers were right and the
// provenance was wrong, and nothing failed. Deriving the whole block — chart, denominators
// and provenance line — from `baseline.*.json` removes the class.
//
//   node docs/site/scripts/render-bench-chart.mjs            # print the block
//   node docs/site/scripts/render-bench-chart.mjs --write    # splice it into README.md
//
// `check-docs.mjs` independently VALIDATES what ends up in the README, so the generator is
// a convenience and never the authority: a hand-edit that drifts still fails CI.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "../../..");

/// Families the README does NOT publish per-family, because they measure AUTHORING and
/// SCRIPTING rather than agentic execution. They are still captured every run; the published
/// comparison is scoped to agentic execution so that it compares like with like.
///
/// ⚠ Withheld ≠ hidden. Their task count and their contribution to the suite-wide figure
/// are published as one disclosed aggregate, and `check-docs.mjs` proves the arithmetic
/// reconciles: shown passes + withheld passes must equal the suite-wide numerator. Without
/// that, a filtered table beside an unfiltered aggregate would simply not add up, and a
/// reader could not tell whether a family was omitted or had never been measured.
export const NOT_AGENTIC_CAPABILITY = new Map([
  ["scaffold", "project scaffolding — code authoring, not agentic execution"],
  ["nlauthor", "authoring durable config from natural language — an authoring surface"],
  ["workflow", "running stored workflow definitions — deterministic step kinds, the model is not what is measured"],
  ["script", "script execution — a runtime capability rather than an agentic one"],
]);

/// Passes a floor-folded per-mille implies over `n` tasks. `task_success` is binary per
/// task, so `floor(1000 * passes / n)` inverts exactly at these sample sizes.
export function passesFrom(perMille, n) {
  return Math.round((perMille * n) / 1000);
}

/// The withheld aggregate: how many tasks the OSS README does not break down, and how many
/// of them passed on each engine. Derived from the same baselines everything else is.
export function withheldAggregate(counts, gates) {
  let tasks = 0;
  const passes = gates.map(() => 0);
  for (const [family, n] of counts) {
    if (!NOT_AGENTIC_CAPABILITY.has(family)) continue;
    tasks += n;
    gates.forEach((g, i) => {
      passes[i] += passesFrom(g.get(`task_success@${family}`) ?? 0, n);
    });
  }
  return { tasks, passes };
}

const ENGINES = [
  ["Ollama", "ollama"],
  ["llama.cpp", "llamacpp"],
];

function baseline(key) {
  return JSON.parse(
    readFileSync(join(REPO, `crates/kx-eval/corpus/bench-v1/baseline.${key}.json`), "utf8"),
  );
}

function suite() {
  return JSON.parse(readFileSync(join(REPO, "crates/kx-eval/corpus/bench-v1/suite.json"), "utf8"));
}


// ---------------------------------------------------------------------------------
// The GROUPED-BAR SVG (owner-selected, 2026-08-04)
// ---------------------------------------------------------------------------------
//
// Mermaid's `xychart-beta` OVERLAYS multiple series rather than grouping them and emits no
// legend, so a shorter bar can sit entirely behind a taller one and a true 0 draws as no
// bar at all — indistinguishable from a family that was never scored. For a chart whose
// whole job is comparing two engines, that is the wrong instrument.
//
// GitHub renders committed SVG in markdown but not styled HTML, so the grouped chart is
// GENERATED here and committed. Two variants (light/dark) drive a `<picture>` element, and
// `check-docs.mjs` reads the numbers back out of the SVG and holds them to the baselines —
// a generated image that nothing validates is a screenshot, and screenshots go stale.

const THEMES = {
  light: { ink: "#141a1f", muted: "#59636e", rule: "#dde3e9", grid: "#eef2f5",
           a: "#17696a", b: "#8e4a68" },
  dark:  { ink: "#e7ecf1", muted: "#94a1ad", rule: "#242e39", grid: "#1b232c",
           a: "#4fb3ac", b: "#d98cae" },
};

const SVG_W = 900, LABEL_W = 165, VALUE_W = 62, BAR_H = 11, BAR_GAP = 4, ROW_GAP = 13;
const PLOT_X = LABEL_W + 14;
const PLOT_W = SVG_W - PLOT_X - VALUE_W - 14;
const HEAD_H = 58;

const esc = (t) => String(t).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/// One grouped-bar SVG. `rows` is `[family, n, ollamaPerMille, llamacppPerMille]`.
export function renderSvg(rows, theme, meta) {
  const c = THEMES[theme];
  const rowH = BAR_H * 2 + BAR_GAP;
  const H = HEAD_H + rows.length * (rowH + ROW_GAP) + 30;
  const mono = "ui-monospace,SFMono-Regular,Menlo,Consolas,monospace";
  const out = [];

  out.push(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${SVG_W} ${H}" width="${SVG_W}" ` +
      `height="${H}" role="img" aria-labelledby="t d" font-family="${mono}">`,
    `<title id="t">Agentic task_success by family, Ollama versus llama.cpp</title>`,
    `<desc id="d">${esc(meta.desc)}</desc>`,
    `<defs><pattern id="z" width="6" height="6" patternTransform="rotate(45)" ` +
      `patternUnits="userSpaceOnUse"><line x1="0" y1="0" x2="0" y2="6" ` +
      `stroke="${c.muted}" stroke-width="2" opacity=".45"/></pattern></defs>`,
  );

  // Legend — the thing mermaid could not give us.
  let lx = PLOT_X;
  for (const [label, fill] of [[meta.legendA, c.a], [meta.legendB, c.b]]) {
    out.push(
      `<rect x="${lx}" y="12" width="20" height="10" rx="2" fill="${fill}"/>`,
      `<text x="${lx + 27}" y="21" font-size="11.5" fill="${c.ink}">${esc(label)}</text>`,
    );
    lx += 34 + label.length * 6.4;
  }
  out.push(
    `<rect x="${lx}" y="12" width="20" height="10" rx="2" fill="url(#z)"/>`,
    `<text x="${lx + 27}" y="21" font-size="11.5" fill="${c.muted}">scored zero</text>`,
    `<line x1="0" y1="36" x2="${SVG_W}" y2="36" stroke="${c.rule}" stroke-width="1"/>`,
  );

  // Quartile gridlines, behind the bars.
  for (const q of [0.25, 0.5, 0.75, 1]) {
    const x = PLOT_X + PLOT_W * q;
    out.push(
      `<line x1="${x.toFixed(1)}" y1="${HEAD_H - 12}" x2="${x.toFixed(1)}" y2="${H - 22}" ` +
        `stroke="${c.grid}" stroke-width="1"/>`,
      `<text x="${x.toFixed(1)}" y="${H - 8}" font-size="10" fill="${c.muted}" ` +
        `text-anchor="middle">${q * 1000}</text>`,
    );
  }

  rows.forEach(([family, n, va, vb], i) => {
    const y = HEAD_H + i * (rowH + ROW_GAP);
    // Machine-readable per-row data. A generated image that nothing validates is a
    // screenshot, and screenshots go stale silently; `check-docs.mjs` reads these
    // attributes back and holds them to the committed baselines.
    out.push(
      `<g data-family="${esc(family)}" data-n="${n}" data-a="${va}" data-b="${vb}">`,
    );
    out.push(
      `<text x="${LABEL_W}" y="${y + 11}" font-size="12" fill="${c.ink}" ` +
        `text-anchor="end">${esc(family)}</text>`,
      `<text x="${LABEL_W}" y="${y + 24}" font-size="10.5" fill="${c.muted}" ` +
        `text-anchor="end">n=${n}</text>`,
    );
    [[va, c.a, 0], [vb, c.b, BAR_H + BAR_GAP]].forEach(([v, fill, dy]) => {
      const by = y + dy;
      if (v === 0) {
        out.push(
          `<rect x="${PLOT_X}" y="${by}" width="${PLOT_W}" height="${BAR_H}" rx="2" ` +
            `fill="url(#z)"/>`,
        );
      } else {
        const w = Math.max(2, (PLOT_W * v) / 1000);
        out.push(
          `<rect x="${PLOT_X}" y="${by}" width="${w.toFixed(1)}" height="${BAR_H}" rx="2" ` +
            `fill="${fill}"/>`,
        );
      }
    });
    out.push(
      `<text x="${SVG_W - 12}" y="${y + 10}" font-size="11" fill="${c.a}" ` +
        `text-anchor="end">${va}</text>`,
      `<text x="${SVG_W - 12}" y="${y + 10 + BAR_H + BAR_GAP}" font-size="11" fill="${c.b}" ` +
        `text-anchor="end">${vb}</text>`,
      `</g>`,
    );
  });

  out.push("</svg>");
  return out.join("\n") + "\n";
}

/// Everything both outputs derive from, computed once from the committed baselines.
function model() {
  const tasks = suite().tasks ?? [];
  const counts = new Map();
  for (const t of tasks) counts.set(t.family, (counts.get(t.family) ?? 0) + 1);

  const loaded = ENGINES.map(([, key]) => baseline(key));
  const gates = loaded.map(
    (b) => new Map((b.gates ?? []).map((g) => [g.id, g.per_mille])),
  );

  // Order by the FIRST engine's score descending, then by name — so the comparison reads
  // as a capability profile rather than as corpus order, and the two series stay aligned.
  const families = [...counts.keys()]
    .filter((f) => !NOT_AGENTIC_CAPABILITY.has(f))
    .sort((a, b) => {
      const d = (gates[0].get(`task_success@${b}`) ?? 0) - (gates[0].get(`task_success@${a}`) ?? 0);
      return d !== 0 ? d : a.localeCompare(b);
    });

  const labels = families.map((f) => `"${f} (n=${counts.get(f)})"`).join(", ");
  const series = gates.map((g) =>
    families.map((f) => g.get(`task_success@${f}`) ?? 0).join(", "),
  );

  const suiteWide = gates.map((g) => g.get("task_success") ?? 0);
  const env = loaded.map((b) => b.env ?? {});
  const sha = (env[0].git_sha ?? "").slice(0, 12);
  const captured = env[0].captured_unix_s
    ? new Date(env[0].captured_unix_s * 1000).toISOString().slice(0, 10)
    : "an unrecorded date";

  const excluded = [...NOT_AGENTIC_CAPABILITY.entries()]
    .map(([f, why]) => `\`${f}\` — ${why}`)
    .join("; ");
  const withheld = withheldAggregate(counts, gates);
  const shown = gates.map((g) =>
    families.reduce((acc, f) => acc + passesFrom(g.get(`task_success@${f}`) ?? 0, counts.get(f)), 0),
  );
  const shownTasks = families.reduce((acc, f) => acc + counts.get(f), 0);
  // The headline aggregate is over the PLOTTED families only, so the number a reader sees
  // is the number the chart above it explains. The whole-suite figure stays in the
  // suite-wide table, where its 46-task denominator is visible.
  const agentic = shown.map((p) => Math.floor((1000 * p) / shownTasks));

  const rows = families.map((f) => [
    f,
    counts.get(f),
    gates[0].get(`task_success@${f}`) ?? 0,
    gates[1].get(`task_success@${f}`) ?? 0,
  ]);
  const meta = {
    legendA: `${ENGINES[0][0]} ${env[0].model ?? "?"}`,
    legendB: `${ENGINES[1][0]} ${env[1].model ?? "?"}`,
    desc:
      `task_success per-mille by capability family, ${ENGINES[0][0]} versus ${ENGINES[1][0]}, ` +
      `captured at ${sha}. Agentic aggregate ${agentic[0]} versus ${agentic[1]} over ` +
      `${shownTasks} tasks.`,
  };

  return {
    families, counts, gates, env, sha, captured, labels, series, suiteWide,
    excluded, withheld, shown, shownTasks, agentic, rows, meta,
  };
}

/// Write the light and dark grouped-bar SVGs the README's `<picture>` points at.
export function writeSvgs() {
  const m = model();
  const dir = join(REPO, "docs/assets");
  mkdirSync(dir, { recursive: true });
  for (const theme of ["light", "dark"]) {
    writeFileSync(join(dir, `bench-agentic-${theme}.svg`), renderSvg(m.rows, theme, m.meta));
  }
  return m;
}

export function renderBlock() {
  const {
    families, env, sha, captured, suiteWide, excluded, withheld, shown, shownTasks,
    agentic, meta,
  } = model();

  return `<!-- bench-chart:comparison — GENERATED by docs/site/scripts/render-bench-chart.mjs
     from crates/kx-eval/corpus/bench-v1/baseline.*.json and validated by
     docs/site/scripts/check-docs.mjs. Series order: ${ENGINES.map(([n]) => n).join(", ")}.
     Keep this anchor. -->
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/bench-agentic-dark.svg">
  <img alt="${meta.desc}" src="docs/assets/bench-agentic-light.svg" width="900">
</picture>

**First series ${ENGINES[0][0]} \`${env[0].model ?? "?"}\`, second ${ENGINES[1][0]} \`${env[1].model ?? "?"}\`.**
**Agentic \`task_success\`: ${agentic[0]} (${shown[0]}/${shownTasks}) on ${ENGINES[0][0]} vs
${agentic[1]} (${shown[1]}/${shownTasks}) on ${ENGINES[1][0]}** — per-mille, over the families
plotted above. Captured at \`${sha}\` on ${captured}; \`n\` is the number of tasks behind each
bar, and a family with a small \`n\` moves in large steps — one task flipping is the whole bar.

**Scope, stated so the arithmetic reconciles.** These ${families.length} agentic families are
${shownTasks} of the suite's ${shownTasks + withheld.tasks} tasks. The remaining
**${withheld.tasks} authoring and scripting tasks** are measured but not broken down here
(${withheld.passes[0]} and ${withheld.passes[1]} passes respectively): ${excluded}. Add them
back and you get the suite-wide **${suiteWide[0]}** / **${suiteWide[1]}** in the table below —
CI checks that sum, so a withheld family can never become an unmeasured one.
`;
}

// Only ACT when run as a command. `check-docs.mjs` imports this module for the shared
// NOT_AGENTIC_CAPABILITY declaration, and a top-level side effect would make the checker
// print a benchmark chart into its own output.
const isCli = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (!isCli) {
  // imported for the declaration only
} else if (process.argv.includes("--write")) {
  writeSvgs();
  const block = renderBlock();
  const path = join(REPO, "README.md");
  const readme = readFileSync(path, "utf8");
  const re =
    /<!--\s*bench-chart:comparison\b[\s\S]*?-->\s*(?:```mermaid\n[\s\S]*?```|<picture>[\s\S]*?<\/picture>)\n[\s\S]*?(?=\n### |\n## |$)/;
  if (!re.test(readme)) {
    console.error("README.md has no bench-chart:comparison block to replace");
    process.exit(1);
  }
  writeFileSync(path, readme.replace(re, block));
  console.log("README.md benchmark comparison regenerated from the committed baselines.");
} else {
  process.stdout.write(renderBlock());
}
