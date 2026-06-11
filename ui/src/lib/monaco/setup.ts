/**
 * Configure a SELF-HOSTED, OFFLINE Monaco singleton — the load-bearing switch for
 * the embedded web console, which has no network at runtime.
 *
 * The default `@monaco-editor/react` loader fetches Monaco from the jsdelivr CDN.
 * `loader.config({ monaco })` overrides that to use a LOCALLY-bundled instance, so
 * `kx serve`'s zero-node console works with no egress. We import only the JSON
 * language (+ its worker) and the base editor worker — plaintext needs no
 * contribution — keeping the lazy chunk as small as Monaco allows.
 *
 * This module is imported ONLY from `MonacoEditorImpl.tsx`, which is itself reached
 * only through `lazy(() => import("./MonacoEditorImpl"))`. So the whole Monaco graph
 * (this file + the ESM editor + the `?worker` chunks) stays a LAZY chunk and never
 * enters the eager modulepreload set the bundle-size gate measures.
 */

import { loader } from "@monaco-editor/react";
// The tree-shakeable ESM API entry (NOT the `monaco-editor` barrel, which drags
// every language). The JSON contribution adds the JSON language + diagnostics.
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import "monaco-editor/esm/vs/language/json/monaco.contribution";
// `?worker` makes Vite emit each worker as its OWN hash-named chunk, loaded at
// runtime by the editor — never a modulepreload, never in the eager set.
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import JsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";

/** The console's light Monaco theme, mapped to the app's locked light tokens
 *  (hard-coded hexes from `app.css` — reading CSS vars at define time is fragile). */
const KX_LIGHT = "kx-light";

let configured = false;

/** Idempotent: wire the offline workers, point `@monaco-editor/react` at the bundled
 *  instance, and register the `kx-light` theme. Safe to call from every editor mount. */
export function configureMonacoOnce(): void {
  if (configured) {
    return;
  }
  configured = true;

  // Monaco creates language services in Web Workers; without this it falls back to
  // a (CDN) default. Same-origin `?worker` chunks keep it fully offline.
  (self as unknown as { MonacoEnvironment: monaco.Environment }).MonacoEnvironment = {
    getWorker(_workerId: string, label: string): Worker {
      return label === "json" ? new JsonWorker() : new EditorWorker();
    },
  };

  monaco.editor.defineTheme(KX_LIGHT, {
    base: "vs",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#ffffff",
      "editor.foreground": "#0d0d0d",
      "editorLineNumber.foreground": "#0d0d0d46",
      "editorLineNumber.activeForeground": "#0d0d0dad",
      "editor.selectionBackground": "#f0450022",
      "editor.lineHighlightBackground": "#0d0d0d08",
      "editorCursor.foreground": "#d83c00",
      "editorIndentGuide.background1": "#0d0d0d14",
      focusBorder: "#f04500",
    },
  });

  // THE switch: use the bundled monaco, never the CDN.
  loader.config({ monaco });
}

export { KX_LIGHT, monaco };
