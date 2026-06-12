/**
 * Pending chat attachments (Batch A): pick files → upload each via `PutContent`
 * → hold the SERVER-derived refs until send. Previews are `blob:` object URLs
 * of the user's OWN files (revoked on remove/clear — no leak). A re-pick of
 * identical bytes collapses onto the existing chip (content-addressed: the
 * server returns the SAME ref with `deduplicated`).
 */

import { useCallback, useRef, useState } from "react";
import { toUiError } from "./errors";
import { usePutContent } from "./use-put-content";

export type AttachmentStatus = "uploading" | "ready" | "failed";

export interface PendingAttachment {
  readonly id: string;
  readonly filename: string;
  readonly mediaType: string;
  readonly size: number;
  readonly objectUrl: string;
  readonly status: AttachmentStatus;
  /** The server-derived 64-hex ref (set once the upload lands). */
  readonly ref?: string;
  /** True iff the server already held identical bytes (advisory badge). */
  readonly deduplicated?: boolean;
  readonly error?: string;
}

export interface UseAttachments {
  readonly attachments: readonly PendingAttachment[];
  /** True while any upload is still in flight (hold the send). */
  readonly uploading: boolean;
  addFiles(files: ArrayLike<File>): void;
  remove(id: string): void;
  clear(): void;
}

export function useAttachments(): UseAttachments {
  const [attachments, setAttachments] = useState<readonly PendingAttachment[]>([]);
  const put = usePutContent();
  // The mutation object identity changes per render; the stable mutateAsync ref
  // keeps addFiles' identity stable for the composer.
  const putRef = useRef(put);
  putRef.current = put;

  const addFiles = useCallback((files: ArrayLike<File>): void => {
    for (const file of Array.from(files)) {
      const id = crypto.randomUUID();
      const objectUrl = URL.createObjectURL(file);
      setAttachments((prev) => [
        ...prev,
        {
          id,
          filename: file.name,
          mediaType: file.type,
          size: file.size,
          objectUrl,
          status: "uploading",
        },
      ]);
      void (async () => {
        try {
          const payload = new Uint8Array(await file.arrayBuffer());
          const result = await putRef.current.mutateAsync({
            payload,
            mediaType: file.type,
            filename: file.name,
          });
          setAttachments((prev) => {
            // Content-addressed collapse: identical bytes already attached ⇒
            // drop this duplicate chip and badge the existing one.
            const existing = prev.find((a) => a.id !== id && a.ref === result.contentRef);
            if (existing) {
              URL.revokeObjectURL(objectUrl);
              return prev
                .filter((a) => a.id !== id)
                .map((a) => (a.id === existing.id ? { ...a, deduplicated: true } : a));
            }
            return prev.map((a) =>
              a.id === id
                ? {
                    ...a,
                    status: "ready" as const,
                    ref: result.contentRef,
                    deduplicated: result.deduplicated,
                  }
                : a,
            );
          });
        } catch (e) {
          const ui = toUiError(e);
          setAttachments((prev) =>
            prev.map((a) => (a.id === id ? { ...a, status: "failed", error: ui.message } : a)),
          );
        }
      })();
    }
  }, []);

  const remove = useCallback((id: string): void => {
    setAttachments((prev) => {
      const gone = prev.find((a) => a.id === id);
      if (gone) {
        URL.revokeObjectURL(gone.objectUrl);
      }
      return prev.filter((a) => a.id !== id);
    });
  }, []);

  const clear = useCallback((): void => {
    setAttachments((prev) => {
      for (const a of prev) {
        URL.revokeObjectURL(a.objectUrl);
      }
      return [];
    });
  }, []);

  return {
    attachments,
    uploading: attachments.some((a) => a.status === "uploading"),
    addFiles,
    remove,
    clear,
  };
}
