/**
 * The console's inline-SVG glyph set (no icon-library dependency — keeps the bundle
 * lean and the icons themeable via `currentColor`). Nav glyphs map 1:1 to the
 * `IconName`s in `nav-model.ts`; the rest are chrome (menu/refresh/etc). An unknown
 * name renders a neutral dot rather than crashing.
 */

import type { SVGProps } from "react";

export type Glyph =
  | "activity"
  | "attach"
  | "history"
  | "chat"
  | "chevron-right"
  | "context"
  | "branches"
  | "moon"
  | "runs"
  | "recipes"
  | "artifacts"
  | "datasets"
  | "tools"
  | "models"
  | "sun"
  | "systems"
  | "menu"
  | "plus"
  | "power"
  | "refresh"
  | "search"
  | "send"
  | "stop"
  | "settings"
  | "terminal"
  | "copy"
  | "download"
  | "share"
  | "external-link"
  | "calendar"
  | "thumb-up"
  | "thumb-down";

// 24×24 viewBox, stroke = currentColor. Multi-subpath `d` is fine.
const PATHS: Record<Glyph, string> = {
  activity: "M3 12h4l2 6 4-15 2 9h6",
  attach:
    "M21 12.5l-8.5 8.5a6 6 0 01-8.5-8.5L12.5 4a4 4 0 015.7 5.7L9.7 18.2a2 2 0 01-2.9-2.9L15 7",
  history: "M3 12a9 9 0 109-9 9.7 9.7 0 00-7 3.2M3 4v4h4M12 7v5l3.5 2",
  chat: "M4 5h16v11H9l-4 4v-4H4z",
  "chevron-right": "M9 6l6 6-6 6",
  runs: "M6 4v16l13-8z",
  recipes: "M6 3h9l3 3v15H6zM9 9h6M9 13h6M9 17h4",
  artifacts: "M3 7l9-4 9 4-9 4-9-4zm0 0v10l9 4 9-4V7M12 11v10",
  datasets:
    "M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zm0 0v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3",
  tools:
    "M14.7 6.3a1 1 0 000 1.4l1.6 1.6a1 1 0 001.4 0l3.77-3.77a6 6 0 01-7.94 7.94l-6.91 6.91a2.12 2.12 0 01-3-3l6.91-6.91a6 6 0 017.94-7.94l-3.76 3.76z",
  models: "M6 6h12v12H6zM9 9h6v6H9zM9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2",
  systems:
    "M6 7a2 2 0 100-4 2 2 0 000 4zm12 0a2 2 0 100-4 2 2 0 000 4zM12 21a2 2 0 100-4 2 2 0 000 4zM7.5 6.5l3.5 9M16.5 6.5L13 15.5",
  context: "M12 3l9 5-9 5-9-5 9-5zM3 12.5l9 5 9-5M3 17l9 5 9-5",
  // A git-branch glyph: a trunk with a forked node (D155 branches).
  branches:
    "M6 3v12M6 15a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM6 6a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zm0 0a9 9 0 0 1-9 9",
  sun: "M12 7a5 5 0 100 10 5 5 0 000-10zm0-5v2m0 16v2M2 12h2m16 0h2M4.9 4.9l1.4 1.4m11.4 11.4l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
  moon: "M21 12.8A9 9 0 1111.2 3 7 7 0 0021 12.8z",
  menu: "M4 6h16M4 12h16M4 18h16",
  plus: "M12 5v14M5 12h14",
  power: "M12 3v8M17.7 6.3a8 8 0 11-11.4 0",
  refresh: "M20 11a8 8 0 10-2.3 6.3M20 6v5h-5",
  search: "M11 19a8 8 0 100-16 8 8 0 000 16zm10 2l-5-5",
  send: "M4 12l16-7-7 16-2-7z",
  stop: "M8 8h8v8H8z",
  settings:
    "M12 9a3 3 0 100 6 3 3 0 000-6zM19 12a7 7 0 00-.1-1l2-1.5-2-3.4-2.3 1a7 7 0 00-1.7-1l-.3-2.6h-4l-.3 2.6a7 7 0 00-1.7 1l-2.3-1-2 3.4 2 1.5a7 7 0 000 2l-2 1.5 2 3.4 2.3-1a7 7 0 001.7 1l.3 2.6h4l.3-2.6a7 7 0 001.7-1l2.3 1 2-3.4-2-1.5a7 7 0 00.1-1z",
  terminal: "M4 17l6-5-6-5M12 19h8",
  copy: "M9 9h11a2 2 0 012 2v9a2 2 0 01-2 2h-9a2 2 0 01-2-2v-9a2 2 0 012-2zM5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1",
  download: "M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4M7 10l5 5 5-5M12 15V3",
  share:
    "M18 8a3 3 0 100-6 3 3 0 000 6zm0 14a3 3 0 100-6 3 3 0 000 6zM6 15a3 3 0 100-6 3 3 0 000 6zm2.6-1.3l6.8 4m0-11.4l-6.8 4",
  "external-link": "M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6M15 3h6v6M10 14L21 3",
  calendar: "M3 6a2 2 0 012-2h14a2 2 0 012 2v13a2 2 0 01-2 2H5a2 2 0 01-2-2zm0 4h18M8 2v4m8-4v4",
  "thumb-up":
    "M14 9V5a3 3 0 00-3-3l-4 9v11h11.28a2 2 0 002-1.7l1.38-9a2 2 0 00-2-2.3zM7 22H4a2 2 0 01-2-2v-7a2 2 0 012-2h3",
  "thumb-down":
    "M10 15v4a3 3 0 003 3l4-9V2H5.72a2 2 0 00-2 1.7l-1.38 9a2 2 0 002 2.3zM17 2h2.67A2.31 2.31 0 0122 4v7a2.31 2.31 0 01-2.33 2H17",
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
