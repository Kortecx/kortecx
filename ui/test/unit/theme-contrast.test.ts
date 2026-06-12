/**
 * The AA contrast audit (PR-0 dual theme) — the D136/D137 contrast lock, executable.
 *
 * Parses BOTH palettes straight out of `src/styles/app.css` (the single styling
 * source) and asserts WCAG 2.x contrast on every text-bearing token pair. The NEW
 * dark palette is held to the full ≥4.5:1 everywhere. A handful of LIGHT pairs
 * shipped below 4.5 under the locked D137 palette (tertiary labels, semantic badge
 * text, accents on the tinted page bg); PR-0 does not retune the locked light set —
 * those pairs are pinned at their shipped floors so they can only improve.
 */

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const CSS_PATH = resolve(dirname(fileURLToPath(import.meta.url)), "../../src/styles/app.css");

// ---------------------------------------------------------------------------
// Tiny color math (sRGB → WCAG relative luminance → contrast ratio).
// ---------------------------------------------------------------------------

interface Rgba {
  readonly r: number; // 0..255
  readonly g: number;
  readonly b: number;
  readonly a: number; // 0..1
}

function parseColor(value: string): Rgba | null {
  const v = value.trim();
  const hex = /^#([0-9a-f]{6})$/i.exec(v);
  if (hex?.[1] !== undefined) {
    const n = Number.parseInt(hex[1], 16);
    return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff, a: 1 };
  }
  const rgba = /^rgba\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*\)$/i.exec(v);
  if (rgba) {
    const [, r, g, b, a] = rgba;
    if (r !== undefined && g !== undefined && b !== undefined && a !== undefined) {
      return { r: Number(r), g: Number(g), b: Number(b), a: Number(a) };
    }
  }
  return null;
}

/** Alpha-composite `fg` over an OPAQUE `bg`. */
function composite(fg: Rgba, bg: Rgba): Rgba {
  const a = fg.a;
  return {
    r: fg.r * a + bg.r * (1 - a),
    g: fg.g * a + bg.g * (1 - a),
    b: fg.b * a + bg.b * (1 - a),
    a: 1,
  };
}

function channel(c: number): number {
  const s = c / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

function luminance(c: Rgba): number {
  return 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b);
}

/** WCAG contrast of `fg` text over an opaque `bg` (compositing translucent fg). */
function contrast(fg: Rgba, bg: Rgba): number {
  const f = luminance(composite(fg, bg));
  const b = luminance(bg);
  const [hi, lo] = f > b ? [f, b] : [b, f];
  return (hi + 0.05) / (lo + 0.05);
}

// ---------------------------------------------------------------------------
// Palette extraction from app.css.
// ---------------------------------------------------------------------------

type Palette = ReadonlyMap<string, Rgba>;

function parseBlock(css: string, selectorRe: RegExp): Palette {
  const block = selectorRe.exec(css)?.[1];
  expect(block, `selector ${selectorRe} present in app.css`).toBeDefined();
  const tokens = new Map<string, Rgba>();
  for (const m of (block as string).matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)) {
    const [, name, raw] = m;
    if (name === undefined || raw === undefined) {
      continue;
    }
    const color = parseColor(raw);
    if (color) {
      tokens.set(`--${name}`, color);
    }
  }
  return tokens;
}

const css = readFileSync(CSS_PATH, "utf8");
const light = parseBlock(css, /:root\s*\{([^}]*)\}/);
const dark = parseBlock(css, /:root\[data-theme="dark"\]\s*\{([^}]*)\}/);

function tokenOf(palette: Palette, theme: string, name: string): Rgba {
  const c = palette.get(name);
  expect(c, `${theme} palette defines ${name} as a literal color`).toBeDefined();
  return c as Rgba;
}

// ---------------------------------------------------------------------------
// The audited pairs. min.light pins shipped D137 floors; min.dark is the full
// AA bar for the NEW palette (the PR-0 gate: no dark token below 4.5 on text).
// ---------------------------------------------------------------------------

const MOTE_TONES = [
  "--t-pending",
  "--t-scheduled",
  "--t-committed",
  "--t-failed",
  "--t-repudiated",
  "--t-inconsistent",
  "--t-unknown",
] as const;
const ND_TONES = ["--t-pure", "--t-read-only-nondet", "--t-world-mutating"] as const;
const SEMANTIC = ["--success", "--warning", "--error", "--info"] as const;

interface Pair {
  readonly fg: string;
  readonly bg: string;
  readonly min: { readonly light: number; readonly dark: number };
  readonly why: string;
}

const PAIRS: Pair[] = [
  // body / card / elevated / input text
  ...["--bg", "--surface", "--surface-elev", "--bg-input"].map((bg) => ({
    fg: "--text-1",
    bg,
    min: { light: 4.5, dark: 4.5 },
    why: "primary text",
  })),
  ...["--bg", "--surface", "--surface-elev", "--bg-input"].map((bg) => ({
    fg: "--text-2",
    bg,
    min: { light: 4.5, dark: 4.5 },
    why: "secondary text",
  })),
  // tertiary uppercase micro-labels: shipped at ~3.1 in light (grandfathered)
  ...["--bg", "--surface"].map((bg) => ({
    fg: "--text-3",
    bg,
    min: { light: 3.0, dark: 4.5 },
    why: "tertiary labels",
  })),
  // the text-bearing orange (links, active chips) on cards and the page bg
  {
    fg: "--primary-h",
    bg: "--surface",
    min: { light: 4.5, dark: 4.5 },
    why: "accent text on cards",
  },
  // shipped light value washes to ~4.06 on the tinted #f0f0f0 page bg (grandfathered)
  { fg: "--primary-h", bg: "--bg", min: { light: 4.0, dark: 4.5 }, why: "accent text on page bg" },
  // text over accent surfaces (.bubble--user, .skip-link, pressed view-toggle)
  { fg: "--accent-fg", bg: "--primary-h", min: { light: 4.5, dark: 4.5 }, why: "text on accent" },
  // pills: --pill-fg text over every mote-state tone
  ...MOTE_TONES.map((bg) => ({
    fg: "--pill-fg",
    bg,
    min: { light: 4.5, dark: 4.5 },
    why: "pill text on state tone",
  })),
  // tones as text on cards
  ...[...MOTE_TONES, ...ND_TONES].map((fg) => ({
    fg,
    bg: "--surface",
    min: { light: 4.5, dark: 4.5 },
    why: "tone as text on cards",
  })),
  // tones as text on the page bg: light #f0f0f0 washes the weakest shipped tone
  // (--t-failed #dc2626) to a measured 4.24 — grandfathered, pinned so it can
  // only improve
  ...[...MOTE_TONES, ...ND_TONES].map((fg) => ({
    fg,
    bg: "--bg",
    min: { light: 4.2, dark: 4.5 },
    why: "tone as text on page bg",
  })),
  // semantic badge text: shipped light values bottom out ~3.2 (grandfathered)
  ...SEMANTIC.map((fg) => ({
    fg,
    bg: "--surface",
    min: { light: 3.0, dark: 4.5 },
    why: "semantic badge text",
  })),
];

describe("theme contrast (the executable AA lock)", () => {
  for (const [theme, palette] of [
    ["light", light],
    ["dark", dark],
  ] as const) {
    describe(`${theme} palette`, () => {
      for (const pair of PAIRS) {
        it(`${pair.fg} on ${pair.bg} ≥ ${pair.min[theme]} (${pair.why})`, () => {
          const fg = tokenOf(palette, theme, pair.fg);
          const bg = tokenOf(palette, theme, pair.bg);
          expect(bg.a, `${pair.bg} must be opaque to sit under text`).toBe(1);
          expect(contrast(fg, bg)).toBeGreaterThanOrEqual(pair.min[theme]);
        });
      }
    });
  }

  it("the theme-independent button gradient keeps white text AA at both stops", () => {
    const white = parseColor("#ffffff") as Rgba;
    for (const stop of ["#d83c00", "#c03500"]) {
      expect(contrast(white, parseColor(stop) as Rgba)).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("the dark block overrides every audited token (no silent light fallthrough)", () => {
    const audited = new Set<string>();
    for (const p of PAIRS) {
      audited.add(p.fg);
      audited.add(p.bg);
    }
    for (const name of audited) {
      expect(dark.has(name), `dark palette defines ${name}`).toBe(true);
    }
  });
});
