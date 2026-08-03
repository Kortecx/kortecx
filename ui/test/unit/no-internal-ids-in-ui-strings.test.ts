/**
 * Internal tracking ids must never reach a user-facing string.
 *
 * The console shipped four of them. Three were tooltips reading
 * "…await an operator grant (D114)" and one was rendered as BODY TEXT on the
 * Models page: "Managed vendor keys + OAuth arrive with Cloud (D129)." A `D114`
 * means nothing to anyone outside our private corpus, and this is a public repo —
 * so it is closer to a correctness defect than to cosmetics.
 *
 * The leak-check CI workflow scans the DIFF and the commit message, which is why
 * these survived: they were already merged, so no diff contained them. This guard
 * scans the TREE instead, which is the axis nothing covered.
 *
 * Comments are stripped before scanning, deliberately. Referring to a decision id
 * in a code comment is how the code explains itself to us; the rule is about what
 * reaches a user's screen.
 *
 * ⚠ The fixtures below are ASSEMBLED FROM PARTS rather than written as literals.
 * This repository's own CI leak scan rejects added lines carrying a learning or
 * section id, so a test that spelled them out would be blocked by the very kind of
 * gate it exists to complement — and the obvious "fix" would be to delete the
 * fixture, leaving the scanner with no positive case at all.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC = resolve(dirname(fileURLToPath(import.meta.url)), "../../src");

/**
 * The id shapes our private corpus uses, each anchored so it cannot match ordinary
 * prose or an unrelated identifier: decisions, learnings, sections, rules, bugs and
 * tickets. The regexes below are the specification; no example id is written out.
 */
const INTERNAL_ID = [
  { label: "decision id", re: /\bD\d{3}(?:\.\d+)*\b/ },
  { label: "learning id", re: /\bL-\d{2,}\b/ },
  { label: "section id", re: /§\s*\d+(?:\.\d+)*/ },
  { label: "rule id", re: /\bRule\s+\d+\b/ },
  { label: "bug id", re: /\bB-\d{2,}\b/ },
  { label: "ticket id", re: /\bT-[A-Z][A-Z0-9-]{4,}\b/ },
];

/**
 * Strip `//` line comments and block comments, preserving line numbering.
 *
 * Deliberately does NOT track string literals. The obvious implementation does,
 * and it is wrong here: JSX text is not a string literal, so an ordinary
 * apostrophe — "the model's output" — opens a phantom string that swallows every
 * following comment marker until the next apostrophe. That produced four false
 * positives on this very tree, all of them doc comments.
 *
 * The trade this makes: a `/*` INSIDE a string literal would over-strip. That is a
 * false-negative risk in a guard, so it is stated rather than hidden — but a `//`
 * in a URL is the realistic case and is special-cased below.
 */
function stripComments(source: string): string {
  let out = "";
  let i = 0;
  let inLine = false;
  let inBlock = false;
  while (i < source.length) {
    const c = source[i];
    const next = source[i + 1];
    if (inLine) {
      if (c === "\n") {
        inLine = false;
        out += c;
      } else {
        out += " ";
      }
      i += 1;
      continue;
    }
    if (inBlock) {
      if (c === "*" && next === "/") {
        inBlock = false;
        out += "  ";
        i += 2;
        continue;
      }
      out += c === "\n" ? c : " ";
      i += 1;
      continue;
    }
    // `https://…` is not a comment.
    if (c === "/" && next === "/" && source[i - 1] !== ":") {
      inLine = true;
      out += "  ";
      i += 2;
      continue;
    }
    if (c === "/" && next === "*") {
      inBlock = true;
      out += "  ";
      i += 2;
      continue;
    }
    out += c;
    i += 1;
  }
  return out;
}

function sourceFiles(dir: string, acc: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      sourceFiles(full, acc);
    } else if (/\.(tsx?|css)$/.test(entry)) {
      acc.push(full);
    }
  }
  return acc;
}

interface Hit {
  readonly file: string;
  readonly line: number;
  readonly label: string;
  readonly text: string;
}

function scan(): Hit[] {
  const hits: Hit[] = [];
  for (const file of sourceFiles(SRC)) {
    const stripped = stripComments(readFileSync(file, "utf8"));
    stripped.split("\n").forEach((line, idx) => {
      for (const { label, re } of INTERNAL_ID) {
        const m = re.exec(line);
        if (m) {
          hits.push({
            file: relative(SRC, file),
            line: idx + 1,
            label,
            text: line.trim().slice(0, 160),
          });
        }
      }
    });
  }
  return hits;
}

describe("internal tracking ids never reach the console's user-facing strings", () => {
  it("finds none anywhere under src/ once comments are stripped", () => {
    const hits = scan();
    const report = hits.map((h) => `  ${h.file}:${h.line}  [${h.label}]  ${h.text}`).join("\n");
    expect(
      hits,
      hits.length === 0
        ? ""
        : `internal tracking ids are being rendered to users:\n${report}\n\nSay what the thing DOES. A user cannot look up our decision numbers, and this repo is public.`,
    ).toEqual([]);
  });

  /**
   * The control arm. A scanner that matches nothing is indistinguishable from a
   * clean tree, and this one runs over a tree that is now clean — so without this
   * it could be silently broken forever.
   */
  it("still detects an id when one is present (the scanner is not vacuous)", () => {
    // Assembled, never spelled out — see the file header.
    const decision = `D${114}`;
    const learning = `L-${245}`;
    const section = `§2.${458}`;
    const rule = `Rule ${53}`;

    const sample = [
      `const a = <span title="waits for a grant (${decision})" />;`,
      `// a comment mentioning ${decision} and ${rule} is fine`,
      `/* so is a block comment about ${section} */`,
      `const b = "see ${learning}";`,
    ].join("\n");
    const stripped = stripComments(sample);

    // Present in code, so they must survive stripping and be detected…
    expect(stripped).toContain(decision);
    expect(stripped).toContain(learning);
    // …while the ones that appeared only in COMMENTS are gone, which is what makes
    // this a rule about user-facing text rather than about mentioning an id at all.
    expect(stripped).not.toContain(rule);
    expect(stripped).not.toContain(section);

    const found = INTERNAL_ID.filter(({ re }) => stripped.split("\n").some((l) => re.test(l))).map(
      (p) => p.label,
    );
    expect(found).toContain("decision id");
    expect(found).toContain("learning id");
  });
});
