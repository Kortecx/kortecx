/**
 * Render one decoded content blob by its kind — the shared OSS "Data Lab" viewer.
 * IMAGE / VIDEO / AUDIO render from a `blob:` object URL built off the decoded
 * bytes (never a remote `src`, so there is no outbound-fetch / SSRF surface);
 * MARKDOWN renders through the dependency-free, React-element-only
 * {@link renderMarkdown} (never innerHTML); JSON / TEXT render in the read-only
 * Monaco {@link CodeViewer}; BINARY shows a bounded hex preview. Every kind has a
 * download. Reused by the run-artifact gallery and the dataset hit drawer.
 *
 * The blob URL is created in an effect and revoked on unmount / change — the same
 * lifecycle discipline as `use-upload-preview` / `use-attachments`.
 */

import { useEffect, useState } from "react";
import type { DecodedContent } from "../lib/content-decode";
import { download, downloadBytes } from "../lib/download";
import { EmptyState } from "./EmptyState";
import { renderMarkdown } from "./chat/markdown";
import { CodeViewer } from "./editor/CodeViewer";

/**
 * The CSP prepended to every sandboxed HTML preview: `default-src 'none'` blocks ALL
 * outbound fetches (no tracking pixels / SSRF), while inline styles + data/blob images
 * still render. Belt-and-suspenders with the iframe's empty `sandbox` (which already
 * disables scripts, forms, same-origin, and navigation).
 */
const HTML_CSP =
  "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; font-src data:;\">";

/** Create a `blob:` object URL for `bytes` (revoked on change/unmount). */
function useObjectUrl(bytes: Uint8Array | undefined, mediaType: string | undefined): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!bytes || bytes.length === 0) {
      setUrl(null);
      return;
    }
    const next = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mediaType }));
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [bytes, mediaType]);
  return url;
}

/** A short download filename for a content ref + its kind. */
function downloadName(stem: string, content: DecodedContent): string {
  const ext = content.mediaType?.split("/")[1];
  if (content.kind === "json") {
    return `${stem}.json`;
  }
  // SVG rides the image path with an `image/svg+xml` MIME — name it `.svg`, not the
  // literal subtype `.svg+xml`.
  if (content.mediaType === "image/svg+xml") {
    return `${stem}.svg`;
  }
  if (ext && (content.kind === "image" || content.kind === "video" || content.kind === "audio")) {
    return `${stem}.${ext}`;
  }
  if (content.kind === "html") {
    return `${stem}.html`;
  }
  if (content.kind === "markdown") {
    return `${stem}.md`;
  }
  return `${stem}.txt`;
}

export function AssetViewer({
  content,
  stem = "artifact",
  bodyTestId = "asset-viewer-body",
  showDownload = true,
}: {
  content: DecodedContent;
  /** The download filename stem (e.g. the leading hex of the content ref). */
  stem?: string;
  /** Test id for the rendered text/code body (media bodies carry their own). */
  bodyTestId?: string;
  /** Whether to show the per-asset download control. */
  showDownload?: boolean;
}) {
  const isMedia = content.kind === "image" || content.kind === "video" || content.kind === "audio";
  // A truncated media payload (the 512 KiB batch clamp) can't render as a valid
  // image/clip — show the honest download instead of a broken element.
  const renderable = isMedia && !content.truncated;
  const url = useObjectUrl(renderable ? content.bytes : undefined, content.mediaType);

  const onDownload = () => {
    if (content.bytes) {
      downloadBytes(downloadName(stem, content), content.bytes, content.mediaType);
    } else {
      download(downloadName(stem, content), content.text);
    }
  };

  return (
    <div className="asset-viewer" data-testid="asset-viewer" data-kind={content.kind}>
      {content.kind === "empty" ? (
        <EmptyState title="Empty" detail="This output has no bytes." />
      ) : isMedia && content.truncated ? (
        <EmptyState
          title="Preview too large"
          detail="This media exceeds the inline preview limit — download it to view."
          action={
            <button
              type="button"
              className="linkbtn"
              data-testid="asset-download"
              onClick={onDownload}
            >
              Download
            </button>
          }
        />
      ) : content.kind === "image" ? (
        url ? (
          <img className="asset-viewer__image" src={url} alt="" data-testid="asset-image" />
        ) : (
          <EmptyState title="Loading image…" />
        )
      ) : content.kind === "video" ? (
        url ? (
          // biome-ignore lint/a11y/useMediaCaption: user-supplied media has no track
          <video className="asset-viewer__media" src={url} controls data-testid="asset-video" />
        ) : (
          <EmptyState title="Loading video…" />
        )
      ) : content.kind === "audio" ? (
        url ? (
          // biome-ignore lint/a11y/useMediaCaption: user-supplied media has no track
          <audio className="asset-viewer__media" src={url} controls data-testid="asset-audio" />
        ) : (
          <EmptyState title="Loading audio…" />
        )
      ) : content.kind === "html" ? (
        <iframe
          className="asset-viewer__html"
          title="HTML preview"
          data-testid="asset-html"
          sandbox=""
          srcDoc={HTML_CSP + content.text}
        />
      ) : content.kind === "markdown" ? (
        <div className="asset-viewer__markdown bubble__md" data-testid="asset-markdown">
          {renderMarkdown(content.text)}
        </div>
      ) : (
        <CodeViewer
          value={content.text}
          language={content.kind === "json" ? "json" : "plaintext"}
          testId={bodyTestId}
          ariaLabel="Content"
          height={Math.min(420, Math.max(120, content.text.split("\n").length * 19 + 24))}
        />
      )}
      {showDownload && content.kind !== "empty" ? (
        <div className="asset-viewer__actions">
          <button
            type="button"
            className="linkbtn"
            data-testid="asset-download"
            onClick={onDownload}
          >
            Download
          </button>
        </div>
      ) : null}
    </div>
  );
}
