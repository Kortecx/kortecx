/**
 * Pure classification of a decoded artifact into a display label + glyph (UI-2
 * artifact gallery). Kept separate from `content-decode` (which owns the
 * bytes→safe-shape decision) so the gallery's presentation logic is testable in
 * isolation and image-ready WITHOUT enabling image rendering yet (the demo recipes
 * emit text/JSON only; inline image rendering is a separate, security-reviewed PR
 * — see the UI-2 plan). When it lands, add an `"image"` kind here + a Blob-URL
 * renderer; nothing else in the gallery changes.
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
  binary: { label: "Binary", glyph: "⬚" },
  empty: { label: "Empty", glyph: "∅" },
};

/** Map a `DecodedKind` to its gallery visual (`Binary` for any unknown kind). */
export function artifactKindVisual(kind: DecodedKind): ArtifactKindVisual {
  return VISUALS[kind] ?? VISUALS.binary;
}
