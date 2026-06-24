/**
 * The REAL Monaco diff wrapper (POC-5d agentic-edit review gate). Imports the heavy
 * `@monaco-editor/react` `DiffEditor`; reached ONLY via `lazy()` from
 * {@link DiffViewer}, so it stays a lazy chunk out of the eager bundle. Never import
 * this module statically.
 */

import { DiffEditor } from "@monaco-editor/react";
import { useTheme } from "../../app/use-theme";
import type { MonacoLanguage } from "../../lib/monaco/infer-language";
import { KX_DARK, KX_LIGHT, configureMonacoOnce } from "../../lib/monaco/setup";

configureMonacoOnce();

export interface DiffViewerImplProps {
  readonly original: string;
  readonly modified: string;
  readonly language: MonacoLanguage;
  readonly height?: number | string;
  readonly testId?: string;
  readonly ariaLabel?: string;
}

export default function DiffViewerImpl({
  original,
  modified,
  language,
  height = 320,
  testId,
  ariaLabel,
}: DiffViewerImplProps) {
  const { resolved } = useTheme();
  return (
    <div className="monaco-host" data-testid={testId} aria-label={ariaLabel}>
      <DiffEditor
        original={original}
        modified={modified}
        language={language}
        theme={resolved === "dark" ? KX_DARK : KX_LIGHT}
        height={height}
        options={{
          readOnly: true,
          renderSideBySide: true,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          automaticLayout: true,
          fontFamily: "var(--font-mono)",
          fontSize: 13,
          wordWrap: "on",
        }}
      />
    </div>
  );
}
