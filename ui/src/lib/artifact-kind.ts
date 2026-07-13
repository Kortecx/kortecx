/**
 * Pure classification of a decoded artifact into a display label + glyph (UI-2
 * artifact gallery). Kept separate from `content-decode` (which owns the
 * bytes→safe-shape decision) so the gallery's presentation logic is testable in
 * isolation. The OSS Data Lab viewer (D157) landed inline media rendering — the
 * image/video/audio/markdown kinds map here; the shared `AssetViewer` does the
 * Blob-URL / markdown rendering, and nothing else in the gallery changed.
 */

import type { DecodedKind } from "./content-decode";

export interface ArtifactKindVisual {
  /** A short human label for the artifact's content kind. */
  readonly label: string;
  /** A glyph (inline text, never an icon-font dep) for the gallery card. */
  readonly glyph: string;
}

const VISUALS: Record<DecodedKind, ArtifactKindVisual> = {
  json: { label: "JSON", glyph: "{ }" },
  text: { label: "Text", glyph: "¶" },
  markdown: { label: "Markdown", glyph: "M↓" },
  html: { label: "HTML", glyph: "◇" },
  binary: { label: "Binary", glyph: "⬚" },
  verdict: { label: "Verdict", glyph: "✓" },
  empty: { label: "Empty", glyph: "∅" },
  image: { label: "Image", glyph: "🖼" },
  video: { label: "Video", glyph: "▶" },
  audio: { label: "Audio", glyph: "♪" },
};

/** Map a `DecodedKind` to its gallery visual (`Binary` for any unknown kind). */
export function artifactKindVisual(kind: DecodedKind): ArtifactKindVisual {
  return VISUALS[kind] ?? VISUALS.binary;
}
