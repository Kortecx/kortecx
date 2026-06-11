/**
 * The console's inline-SVG glyph set (no icon-library dependency — keeps the bundle
 * lean and the icons themeable via `currentColor`). Nav glyphs map 1:1 to the
 * `IconName`s in `nav-model.ts`; the rest are chrome (menu/refresh/etc). An unknown
 * name renders a neutral dot rather than crashing.
 */

import type { SVGProps } from "react";

export type Glyph =
  | "activity"
  | "monitor"
  | "chat"
  | "chevron-right"
  | "runs"
  | "recipes"
  | "artifacts"
  | "datasets"
  | "tools"
  | "systems"
  | "menu"
  | "refresh"
  | "search"
  | "send"
  | "settings"
  | "terminal";

// 24×24 viewBox, stroke = currentColor. Multi-subpath `d` is fine.
const PATHS: Record<Glyph, string> = {
  activity: "M3 12h4l2 6 4-15 2 9h6",
  monitor: "M3 3v18h18M8 16V9m4 7V5m4 11v-4",
  chat: "M4 5h16v11H9l-4 4v-4H4z",
  "chevron-right": "M9 6l6 6-6 6",
  runs: "M6 4v16l13-8z",
  recipes: "M6 3h9l3 3v15H6zM9 9h6M9 13h6M9 17h4",
  artifacts: "M3 7l9-4 9 4-9 4-9-4zm0 0v10l9 4 9-4V7M12 11v10",
  datasets:
    "M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zm0 0v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3",
  tools:
    "M14.7 6.3a1 1 0 000 1.4l1.6 1.6a1 1 0 001.4 0l3.77-3.77a6 6 0 01-7.94 7.94l-6.91 6.91a2.12 2.12 0 01-3-3l6.91-6.91a6 6 0 017.94-7.94l-3.76 3.76z",
  systems:
    "M6 7a2 2 0 100-4 2 2 0 000 4zm12 0a2 2 0 100-4 2 2 0 000 4zM12 21a2 2 0 100-4 2 2 0 000 4zM7.5 6.5l3.5 9M16.5 6.5L13 15.5",
  menu: "M4 6h16M4 12h16M4 18h16",
  refresh: "M20 11a8 8 0 10-2.3 6.3M20 6v5h-5",
  search: "M11 19a8 8 0 100-16 8 8 0 000 16zm10 2l-5-5",
  send: "M4 12l16-7-7 16-2-7z",
  settings:
    "M12 9a3 3 0 100 6 3 3 0 000-6zM19 12a7 7 0 00-.1-1l2-1.5-2-3.4-2.3 1a7 7 0 00-1.7-1l-.3-2.6h-4l-.3 2.6a7 7 0 00-1.7 1l-2.3-1-2 3.4 2 1.5a7 7 0 000 2l-2 1.5 2 3.4 2.3-1a7 7 0 001.7 1l.3 2.6h4l.3-2.6a7 7 0 001.7-1l2.3 1 2-3.4-2-1.5a7 7 0 00.1-1z",
  terminal: "M4 17l6-5-6-5M12 19h8",
};

const FALLBACK = "M12 12h.01";

export interface IconProps extends SVGProps<SVGSVGElement> {
  name: string;
  size?: number;
}

export function Icon({ name, size = 18, ...props }: IconProps) {
  const d = PATHS[name as Glyph] ?? FALLBACK;
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      <path d={d} />
    </svg>
  );
}
