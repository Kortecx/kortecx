#!/usr/bin/env node
// Three checks Docusaurus's own build cannot make.
//
// (1) ORPHAN PAGES — a page absent from the hand-authored `sidebars.ts` is
//     unreachable by navigation. Docusaurus builds it happily (it is a valid route),
//     so nothing complains. `llm-rerank.md` sat orphaned this way.
//
// (2) ANCHORS ON LINKS THAT GO THROUGH GITHUB.
//
// Docusaurus resolves and checks its own relative links (`onBrokenLinks` /
// `onBrokenAnchors`), but a link written as an absolute
// `https://github.com/Kortecx/kortecx/blob/main/README.md#cli-reference` is just an
// external URL to it — so a heading can be renamed or deleted and every gate stays
// green while the reader lands at the top of a long file with no idea what they were
// promised. That is how `README.md#cli-reference` survived: the README has never had
// a "CLI reference" heading.
//
// This checks that class, and only that class. Relative doc-to-doc anchors stay
// Docusaurus's job (it understands explicit `{#id}` overrides; a second slugifier
// here would disagree with it eventually).
//
// (3) THE README'S BENCHMARK TABLE vs the committed per-engine baselines — the
//     numbers the project leads with. A re-baseline is deliberate, so the table and
//     the baseline must move together; otherwise the README advertises a score the
//     ratchet no longer holds.
//
// Usage: node scripts/check-docs.mjs        (from docs/site/)

import { readdir, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Not `import.meta.dirname` — that needs Node 20.11, and this package declares
// `engines.node >= 18`.
const SITE = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPO = resolve(SITE, "..", "..");
const BLOB = /https:\/\/github\.com\/Kortecx\/kortecx\/blob\/[^/]+\/([^)\s#]+\.md)#([^)\s"]+)/g;

/** Every `.md` under `dir`, recursively. */
async function markdownUnder(dir) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await markdownUnder(path)));
    else if (entry.name.endsWith(".md")) out.push(path);
  }
  return out;
}

/** Files whose links we check. */
const SOURCES = [
  ...(await markdownUnder(join(SITE, "docs"))),
  join(REPO, "README.md"),
  join(REPO, "bindings/python/README.md"),
  join(REPO, "bindings/typescript/README.md"),
];

/**
 * GitHub's heading slug: lowercase, drop everything that is not a letter, digit,
 * space, hyphen or underscore, then spaces to hyphens. Deliberately literal —
 * "Workflows, chains & forms" keeps the double hyphen the removed `&` leaves behind,
 * because that is the anchor GitHub actually serves.
 */
function slug(heading) {
  return heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N} _-]/gu, "")
    .replace(/ /g, "-");
}

/** Every anchor a markdown file offers, from its ATX headings. */
async function anchorsOf(path) {
  const text = await readFile(path, "utf8");
  const found = new Set();
  const counts = new Map();
  for (const line of text.split("\n")) {
    const m = /^#{1,6}\s+(.*)$/.exec(line);
    if (!m) continue;
    const base = slug(m[1]);
    if (!base) continue;
    // GitHub disambiguates repeats with -1, -2, … — mirror it so a legitimate
    // link to a duplicated heading is not reported as broken.
    const n = counts.get(base) ?? 0;
    counts.set(base, n + 1);
    found.add(n === 0 ? base : `${base}-${n}`);
  }
  return found;
}

const anchorCache = new Map();
const problems = [];

// (1) Every docs page must be listed in the hand-authored sidebar.
{
  const sidebar = await readFile(join(SITE, "sidebars.ts"), "utf8");
  const docsRoot = join(SITE, "docs");
  for (const path of await markdownUnder(docsRoot)) {
    // The sidebar keys pages by their doc id — the repo convention is the path
    // relative to docs/ without the extension (`chains/python`).
    const id = path.slice(docsRoot.length + 1).replace(/\.md$/, "");
    if (!sidebar.includes(`"${id}"`)) {
      problems.push(
        `sidebars.ts: "${id}" is not listed — the page builds but nothing links to it`,
      );
    }
  }
}

// (3) The README's benchmark table must equal the committed per-engine baselines.
// These are the numbers the project leads with, and a re-baseline is a deliberate act
// — so the table and the baseline it quotes have to move together or the README starts
// advertising a score the ratchet no longer holds.
{
  const readme = await readFile(join(REPO, "README.md"), "utf8");
  const engines = [
    ["Ollama", "crates/kx-eval/corpus/bench-v1/baseline.ollama.json"],
    ["llama.cpp", "crates/kx-eval/corpus/bench-v1/baseline.llamacpp.json"],
  ];
  const loaded = [];
  for (const [name, rel] of engines) {
    const path = join(REPO, rel);
    if (!existsSync(path)) {
      problems.push(`${rel}: missing — the README quotes its numbers`);
      continue;
    }
    const doc = JSON.parse(await readFile(path, "utf8"));
    loaded.push([name, new Map(doc.gates.map((g) => [g.id, g.per_mille]))]);
  }

  // The column order is read from the table's own header rather than assumed: comparing
  // a claimed number against the wrong engine's baseline would pass or fail for the
  // wrong reason, and swapping two columns is an easy edit.
  const header = /^\|\s*Family\s*\|[^|]*\|([^|]*)\|([^|]*)\|/m.exec(readme);
  if (!header) {
    problems.push("README.md: benchmark table header not found — cannot verify its numbers");
  } else {
    const declared = [header[1].trim(), header[2].trim()];
    const expected = engines.map(([name]) => name);
    if (declared.join("|") !== expected.join("|")) {
      problems.push(
        `README.md: benchmark columns read ${declared.join(", ")}; this check compares ` +
          `them as ${expected.join(", ")} — reorder the check or the table`,
      );
    }
  }

  // `| **tool** | …prose… | 1000 | 800 |` — the family, then one column per engine in
  // the header's order.
  const rows = [...readme.matchAll(/^\|\s*\*\*(tool|react|reach|swarm)\*\*\s*\|[^|]*\|([^|]*)\|([^|]*)\|/gm)];
  if (rows.length !== 4) {
    problems.push(
      `README.md: expected 4 benchmark family rows (tool/react/reach/swarm), found ${rows.length}`,
    );
  }
  for (const [, family, ...cols] of rows) {
    loaded.forEach(([engine, gate], i) => {
      const claimed = Number(cols[i].trim());
      const actual = gate.get(`task_success@${family}`);
      if (actual === undefined) {
        problems.push(`baseline.${engine}: no task_success@${family} gate, but README quotes one`);
      } else if (claimed !== actual) {
        problems.push(
          `README.md: ${family} on ${engine} reads ${claimed}, baseline says ${actual}`,
        );
      }
    });
  }
  // The prose claim beside the table.
  if (/`groundedness` and `memory_quality` both score \*\*1000\*\*/.test(readme)) {
    for (const [engine, gate] of loaded) {
      for (const id of ["groundedness", "memory_quality"]) {
        if (gate.get(id) !== 1000) {
          problems.push(
            `README.md claims ${id} is 1000, but baseline.${engine} says ${gate.get(id)}`,
          );
        }
      }
    }
  }
}

for (const source of SOURCES) {
  if (!existsSync(source)) continue;
  const text = await readFile(source, "utf8");
  for (const [, relPath, anchor] of text.matchAll(BLOB)) {
    const target = join(REPO, relPath);
    if (!existsSync(target)) {
      problems.push(`${source}: links to ${relPath}, which does not exist in the repo`);
      continue;
    }
    if (!anchorCache.has(target)) anchorCache.set(target, await anchorsOf(target));
    const anchors = anchorCache.get(target);
    if (!anchors.has(anchor)) {
      problems.push(
        `${source}: ${relPath}#${anchor} — no such heading. ` +
          `Closest: ${[...anchors].filter((a) => a.startsWith(anchor.slice(0, 4))).join(", ") || "(none)"}`,
      );
    }
  }
}

if (problems.length > 0) {
  console.error(`✗ ${problems.length} docs problem(s):\n`);
  for (const p of problems) console.error(`  ${p}`);
  console.error("\nList the page in the sidebar · fix the link or add the heading · move the");
  console.error("README table and the baseline together. Each of these fails silently: an");
  console.error("unreachable page, a link that lands at the top of a long file, and a score");
  console.error("the ratchet no longer holds all look fine until someone relies on them.");
  process.exit(1);
}

console.log(
  `✓ every docs page is in the sidebar, every GitHub-hosted anchor resolves, and the ` +
    `README's benchmark table matches the committed baselines (${SOURCES.length} files scanned)`,
);
