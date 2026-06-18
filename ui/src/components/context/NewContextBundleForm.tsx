/**
 * Author (upsert) a context bundle (`PutContextBundle`, PR-7). Items are content
 * refs already in the store: either uploaded here (file → `PutContent` → ref, the
 * chat-attach path) or named by an existing 64-hex ref (the `kx context add
 * --item` power path). The server derives `bundleRef` (SN-8) and folds the bundle
 * into the entry Mote at bind. Mirrors `RegisterToolForm` (GlowCard + chip/inline
 * controls, never a controlled `<select>`).
 */

import { type FormEvent, useRef, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { usePutContextBundle } from "../../kx/use-context-bundles";
import { usePutContent } from "../../kx/use-put-content";
import { DigestChip } from "../DigestChip";
import { GlowCard } from "../ds/GlowCard";

interface StagedItem {
  name: string;
  contentRef: string;
  mediaType: string;
}

/** A content ref is the 32-byte blake3 store id rendered as 64 lowercase hex. */
const HEX64 = /^[0-9a-f]{64}$/;

export function NewContextBundleForm() {
  const [handle, setHandle] = useState("");
  const [description, setDescription] = useState("");
  const [items, setItems] = useState<StagedItem[]>([]);
  const [refName, setRefName] = useState("");
  const [refHex, setRefHex] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const put = usePutContextBundle();
  const upload = usePutContent();

  const canSubmit = handle.trim().length > 0 && items.length > 0 && !upload.isPending;

  function addItem(item: StagedItem): void {
    setItems((prev) =>
      prev.some((p) => p.contentRef === item.contentRef && p.name === item.name)
        ? prev
        : [...prev, item],
    );
  }

  function removeItem(idx: number): void {
    setItems((prev) => prev.filter((_, i) => i !== idx));
  }

  async function onFiles(files: FileList | null): Promise<void> {
    if (!files) {
      return;
    }
    setLocalError(null);
    for (const file of Array.from(files)) {
      try {
        const payload = new Uint8Array(await file.arrayBuffer());
        const result = await upload.mutateAsync({
          payload,
          mediaType: file.type || "application/octet-stream",
          filename: file.name,
        });
        addItem({ name: file.name, contentRef: result.contentRef, mediaType: file.type });
      } catch (e) {
        setLocalError(toUiError(e).message);
      }
    }
    if (fileInput.current) {
      fileInput.current.value = "";
    }
  }

  function onAddRef(): void {
    const hex = refHex.trim().toLowerCase();
    if (!HEX64.test(hex)) {
      setLocalError("A content ref is 64 hex characters (a 32-byte store id).");
      return;
    }
    setLocalError(null);
    addItem({ name: refName.trim(), contentRef: hex, mediaType: "" });
    setRefName("");
    setRefHex("");
  }

  function onSubmit(e: FormEvent): void {
    e.preventDefault();
    if (!canSubmit) {
      return;
    }
    put.mutate(
      {
        handle: handle.trim(),
        description: description.trim(),
        items: items.map((i) => ({
          name: i.name,
          contentRef: i.contentRef,
          mediaType: i.mediaType || undefined,
        })),
      },
      {
        onSuccess: () => {
          setHandle("");
          setDescription("");
          setItems([]);
        },
      },
    );
  }

  const err = put.error ? toUiError(put.error) : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="context-bundle-new">
      <h2>New context bundle</h2>
      <p className="muted">
        Group files or content refs under a handle, then attach it to a chat or chain. The server
        derives the bundle ref (SN-8) and injects the items into the run's entry step — a different
        attached context is a different, independently-cached run.
      </p>
      <form onSubmit={onSubmit} className="register-tool-form">
        <div className="register-tool-form__row">
          <input
            type="text"
            data-testid="context-bundle-handle"
            placeholder="handle (e.g. team/ctx/spec)"
            value={handle}
            onChange={(e) => setHandle(e.target.value)}
            aria-label="Bundle handle"
          />
          <input
            type="text"
            data-testid="context-bundle-description"
            placeholder="description (optional)"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            aria-label="Bundle description"
          />
        </div>

        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Items</legend>
          <div className="context-bundle-form__add">
            <button
              type="button"
              className="chip"
              data-testid="context-bundle-upload"
              disabled={upload.isPending}
              onClick={() => fileInput.current?.click()}
            >
              <span className="chip__label">
                {upload.isPending ? "Uploading…" : "+ Upload file"}
              </span>
            </button>
            <input
              ref={fileInput}
              type="file"
              multiple
              hidden
              data-testid="context-bundle-file-input"
              onChange={(e) => void onFiles(e.target.files)}
            />
          </div>
          <div className="context-bundle-form__ref">
            <input
              type="text"
              data-testid="context-bundle-ref-name"
              placeholder="ref label (optional)"
              value={refName}
              onChange={(e) => setRefName(e.target.value)}
              aria-label="Content ref label"
            />
            <input
              type="text"
              className="mono"
              data-testid="context-bundle-ref-hex"
              placeholder="content ref (64 hex)"
              value={refHex}
              onChange={(e) => setRefHex(e.target.value)}
              aria-label="Content ref (64 hex)"
            />
            <button
              type="button"
              className="chip"
              data-testid="context-bundle-add-ref"
              onClick={onAddRef}
            >
              <span className="chip__label">+ Add ref</span>
            </button>
          </div>

          {items.length > 0 ? (
            <ul className="context-bundle__items" data-testid="context-bundle-staged">
              {items.map((it, idx) => (
                <li key={`${it.name}:${it.contentRef}`} className="context-bundle__item">
                  <span className="context-bundle__item-name">{it.name || "(unnamed)"}</span>
                  <DigestChip hex={it.contentRef} label={it.name} />
                  <button
                    type="button"
                    className="btn-ghost"
                    data-testid={`context-bundle-staged-remove-${idx}`}
                    aria-label={`Remove ${it.name || "item"}`}
                    onClick={() => removeItem(idx)}
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted">No items yet — upload a file or add a content ref.</p>
          )}
        </fieldset>

        <button
          type="submit"
          data-testid="context-bundle-submit"
          disabled={put.isPending || !canSubmit}
        >
          {put.isPending ? "Saving…" : "Save bundle"}
        </button>
      </form>

      {localError ? (
        <p className="field-error" data-testid="context-bundle-local-error" role="alert">
          {localError}
        </p>
      ) : null}
      {err ? (
        <p className="field-error" data-testid="context-bundle-error" role="alert">
          {err.message}
        </p>
      ) : null}
      {put.isSuccess ? (
        <p className="register-tool__result" data-testid="context-bundle-result">
          Saved <code className="mono">{put.data?.handle}</code>
          {put.data?.deduplicated ? " (unchanged — identical manifest)" : ""}
        </p>
      ) : null}
    </GlowCard>
  );
}
