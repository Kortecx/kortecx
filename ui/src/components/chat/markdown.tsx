/**
 * Block-level Markdown → React elements for assistant chat bubbles.
 *
 * NEVER innerHTML: every node is a React element, so model output cannot inject
 * markup — the same safety class as the codebase's "render `.text` as children"
 * rule (`content-decode.ts`, `ArtifactView.tsx`). Covers exactly the subset the
 * chat needs — `#`/`##`/`###` headings, fenced ``` ``` ``` code, `-`/`*` and `1.`
 * lists, `>` blockquote, and the inline run (`**bold**`, `*italic*`/`_italic_`,
 * `` `code` ``, `[text](url)`). No GFM tables (the chat UX doesn't need them);
 * unknown syntax degrades to LITERAL text, never to raw HTML. Links are
 * scheme-allowlisted (http/https/mailto) with `rel="noopener noreferrer"`.
 *
 * Dependency-free (~2 KB) so it fits the eager bundle budget — react-markdown's
 * ~100 KB graph would force a lazy chunk + a first-turn plain-text flash.
 */

import type { ReactNode } from "react";

const SAFE_SCHEME = /^(?:https?:|mailto:)/i;

/** Allow only http(s)/mailto hrefs; drop `javascript:`/`data:` (return undefined). */
function safeHref(url: string): string | undefined {
  const u = url.trim();
  return SAFE_SCHEME.test(u) ? u : undefined;
}

// One inline pass. Order matters: inline `code` first (its body is never
// re-parsed), then [links], then **bold**, then *italic*/_italic_.
const INLINE = /(`[^`]+`)|(\[[^\]]+\]\([^)\s]+\))|(\*\*[^*]+\*\*)|(\*[^*]+\*)|(_[^_]+_)/;

function inlineRun(text: string, keyBase: string): ReactNode[] {
  const out: ReactNode[] = [];
  let rest = text;
  let i = 0;
  while (rest.length > 0) {
    const m = INLINE.exec(rest);
    if (m === null) {
      out.push(rest);
      break;
    }
    if (m.index > 0) {
      out.push(rest.slice(0, m.index));
    }
    const tok = m[0];
    const key = `${keyBase}-${i}`;
    i += 1;
    if (tok.startsWith("`")) {
      out.push(<code key={key}>{tok.slice(1, -1)}</code>);
    } else if (tok.startsWith("[")) {
      const close = tok.indexOf("](");
      const label = tok.slice(1, close);
      const href = safeHref(tok.slice(close + 2, -1));
      out.push(
        href === undefined ? (
          label
        ) : (
          <a key={key} href={href} target="_blank" rel="noopener noreferrer">
            {label}
          </a>
        ),
      );
    } else if (tok.startsWith("**")) {
      out.push(<strong key={key}>{tok.slice(2, -2)}</strong>);
    } else {
      out.push(<em key={key}>{tok.slice(1, -1)}</em>);
    }
    rest = rest.slice(m.index + tok.length);
  }
  return out;
}

const FENCE = /^\s*```/;
const HEADING = /^(#{1,3})\s+(.*)$/;
const QUOTE = /^\s*>\s?/;
const LIST_ITEM = /^\s*([-*]|\d+\.)\s+/;
const ORDERED = /^\s*\d+\.\s+/;

/** Parse `src` into React block elements. Pure + total — any input is safe. */
export function renderMarkdown(src: string): ReactNode {
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let key = 0;
  while (i < lines.length) {
    const line = lines[i] ?? "";

    // Fenced code — body is a TEXT child, never re-parsed for inline or HTML.
    if (FENCE.test(line)) {
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !FENCE.test(lines[i] ?? "")) {
        body.push(lines[i] ?? "");
        i += 1;
      }
      i += 1; // skip the closing fence
      blocks.push(
        <pre key={`b${key}`} className="md-pre">
          <code className="md-code mono">{body.join("\n")}</code>
        </pre>,
      );
      key += 1;
      continue;
    }

    // Heading.
    const h = HEADING.exec(line);
    if (h !== null) {
      const content = inlineRun(h[2] ?? "", `h${key}`);
      const level = (h[1] ?? "#").length;
      blocks.push(
        level === 1 ? (
          <h1 key={`b${key}`}>{content}</h1>
        ) : level === 2 ? (
          <h2 key={`b${key}`}>{content}</h2>
        ) : (
          <h3 key={`b${key}`}>{content}</h3>
        ),
      );
      key += 1;
      i += 1;
      continue;
    }

    // Blockquote (consecutive `>` lines).
    if (QUOTE.test(line)) {
      const quote: string[] = [];
      while (i < lines.length && QUOTE.test(lines[i] ?? "")) {
        quote.push((lines[i] ?? "").replace(QUOTE, ""));
        i += 1;
      }
      blocks.push(<blockquote key={`b${key}`}>{inlineRun(quote.join(" "), `q${key}`)}</blockquote>);
      key += 1;
      continue;
    }

    // Unordered / ordered list (consecutive item lines).
    if (LIST_ITEM.test(line)) {
      const ordered = ORDERED.test(line);
      const items: ReactNode[] = [];
      let n = 0;
      while (i < lines.length && LIST_ITEM.test(lines[i] ?? "")) {
        const item = (lines[i] ?? "").replace(LIST_ITEM, "");
        items.push(<li key={`li${key}-${n}`}>{inlineRun(item, `li${key}-${n}`)}</li>);
        n += 1;
        i += 1;
      }
      blocks.push(ordered ? <ol key={`b${key}`}>{items}</ol> : <ul key={`b${key}`}>{items}</ul>);
      key += 1;
      continue;
    }

    // Blank line → block separator.
    if (line.trim() === "") {
      i += 1;
      continue;
    }

    // Paragraph — gather consecutive non-blank, non-block lines; soft breaks → <br/>.
    const para: string[] = [];
    while (i < lines.length) {
      const l = lines[i] ?? "";
      if (
        l.trim() === "" ||
        FENCE.test(l) ||
        HEADING.test(l) ||
        QUOTE.test(l) ||
        LIST_ITEM.test(l)
      ) {
        break;
      }
      para.push(l);
      i += 1;
    }
    const nodes: ReactNode[] = [];
    let bidx = 0;
    for (const p of para) {
      if (bidx > 0) {
        nodes.push(<br key={`p${key}-br-${bidx}`} />);
      }
      nodes.push(...inlineRun(p, `p${key}-${bidx}`));
      bidx += 1;
    }
    blocks.push(<p key={`b${key}`}>{nodes}</p>);
    key += 1;
  }
  return blocks;
}
