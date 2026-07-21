/**
 * The shared prop contract for every editor surface (the lazy {@link MonacoMount},
 * the real {@link MonacoEditorImpl}, and the headless fallback). One concern: keep
 * the boundary + the impl in lock-step without either importing the heavy graph.
 */

import type { MonacoLanguage } from "../../lib/monaco/infer-language";

export interface EditorSurfaceProps {
  /** The text shown (controlled — the parent owns the value). */
  readonly value: string;
  /** Monaco language id (we ship `json` + `plaintext` only). */
  readonly language: MonacoLanguage;
  /** Read-only viewer (no edits) vs an editable control. */
  readonly readOnly?: boolean;
  /** Edit callback (editable only). */
  readonly onChange?: (value: string) => void;
  /** CSS height (px number or any CSS length). */
  readonly height?: number | string;
  /** Stable test handle (preserved across the Monaco and fallback renders). */
  readonly testId?: string;
  readonly ariaLabel?: string;
  /** DOM id forwarded to the editable fallback `<textarea>` (label association). */
  readonly id?: string;
  /** Plain-Enter submit (the chat composer). Shift+Enter keeps inserting a
   *  newline in BOTH the real editor and the fallback textarea. */
  readonly onSubmit?: () => void;
  /** Ghost text shown while empty (Monaco `placeholder` option / textarea attr). */
  readonly placeholder?: string;
  /**
   * Keep the newest line in view as `value` grows — for a surface fed by a live token
   * stream, where without it the text scrolls past the viewport and the user watches a
   * frozen first screen while the model writes.
   *
   * Sticky, not forced: once the reader scrolls up they stay where they are, and
   * following resumes when they return to the bottom. Yanking someone back mid-read is
   * worse than not following at all.
   */
  readonly followTail?: boolean;
}
