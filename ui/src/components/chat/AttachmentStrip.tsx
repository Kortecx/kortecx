import type { PendingAttachment } from "../../kx/use-attachments";
import { DigestChip } from "../DigestChip";

/**
 * The pending attachments above the composer (Batch A): an image preview from
 * the session-local `blob:` URL of the user's OWN file (never untrusted server
 * bytes), the SERVER-derived ref as a DigestChip once uploaded, a dedup badge
 * ("already on server"), and a remove control. Upload failures stay visible on
 * the chip (the send proceeds without failed attachments only if removed).
 */
export function AttachmentStrip({
  attachments,
  onRemove,
}: {
  attachments: readonly PendingAttachment[];
  onRemove: (id: string) => void;
}) {
  if (attachments.length === 0) {
    return null;
  }
  return (
    <div className="attachstrip" data-testid="attachment-strip">
      {attachments.map((a) => (
        <div
          key={a.id}
          className={`attachstrip__chip${a.status === "failed" ? " attachstrip__chip--failed" : ""}`}
          data-testid="attachment-chip"
          data-status={a.status}
        >
          {a.mediaType.startsWith("image/") ? (
            <img className="attachstrip__preview" src={a.objectUrl} alt={a.filename} />
          ) : null}
          <span className="attachstrip__name" title={a.filename}>
            {a.filename}
          </span>
          {a.status === "uploading" ? <span className="muted">uploading…</span> : null}
          {a.status === "failed" ? (
            <span className="attachstrip__error" title={a.error}>
              upload failed
            </span>
          ) : null}
          {a.ref ? <DigestChip hex={a.ref} label={a.filename} /> : null}
          {a.deduplicated ? (
            <span className="attachstrip__dedup" data-testid="attachment-dedup">
              already on server
            </span>
          ) : null}
          <button
            type="button"
            className="iconbtn attachstrip__remove"
            onClick={() => onRemove(a.id)}
            aria-label={`Remove ${a.filename}`}
            data-testid="attachment-remove"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
