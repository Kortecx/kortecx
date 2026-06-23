/**
 * One context-bundle item with a POC-2 view/edit affordance. Collapsed it shows
 * the advisory name + media + content `DigestChip`; expanded it lazily fetches the
 * FULL body ({@link useContextItemBody}) and renders it through the shared
 * read-only {@link AssetViewer}. Text-like items (json / text / markdown / empty,
 * not truncated) can be edited in {@link TextEditor} and saved — CAS is immutable,
 * so a save uploads new bytes and re-points this item (a guarded re-upsert). Items
 * can be renamed and removed. Binary / media / truncated bodies are honestly
 * download-only (no fake edit, D142). Every mutation carries the viewed `bundleRef`
 * so a concurrent change is refused, never silently clobbered.
 */

import { useState } from "react";
import { toUiError } from "../../kx/errors";
import {
  useEditContextItem,
  useRemoveContextItem,
  useRenameContextItem,
} from "../../kx/use-context-bundles";
import { useContextItemBody } from "../../kx/use-context-item";
import type { MonacoLanguage } from "../../lib/monaco/infer-language";
import { AssetViewer } from "../AssetViewer";
import { DigestChip } from "../DigestChip";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { TextEditor } from "../editor/TextEditor";

const EDITABLE_KINDS = new Set(["json", "text", "markdown", "empty"]);

function editorLanguage(kind: string): MonacoLanguage {
  if (kind === "json") {
    return "json";
  }
  if (kind === "markdown") {
    return "markdown";
  }
  return "plaintext";
}

export function ContextItemRow({
  handle,
  bundleRef,
  index,
  name,
  contentRef,
  mediaType,
  itemCount,
}: {
  handle: string;
  bundleRef: string;
  index: number;
  name: string;
  contentRef: string;
  mediaType: string;
  itemCount: number;
}) {
  const [expanded, setExpanded] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [renaming, setRenaming] = useState(false);
  const [nameDraft, setNameDraft] = useState(name);

  const body = useContextItemBody(contentRef, mediaType, name, expanded);
  const edit = useEditContextItem();
  const rename = useRenameContextItem();
  const remove = useRemoveContextItem();

  const decoded = body.data;
  const editable = decoded !== undefined && EDITABLE_KINDS.has(decoded.kind) && !decoded.truncated;
  const mutationError = edit.error ?? rename.error ?? remove.error;
  const testKey = `${handle}-${index}`;

  const startEditing = () => {
    if (decoded) {
      setDraft(decoded.text);
      setEditing(true);
    }
  };

  const save = () => {
    edit.mutate(
      { handle, itemIndex: index, text: draft, mediaType, expectBundleRef: bundleRef },
      { onSuccess: () => setEditing(false) },
    );
  };

  const submitRename = () => {
    const next = nameDraft.trim();
    if (next === "" || next === name) {
      setRenaming(false);
      return;
    }
    rename.mutate(
      { handle, itemIndex: index, newName: next, expectBundleRef: bundleRef },
      { onSuccess: () => setRenaming(false) },
    );
  };

  return (
    <li className="context-bundle__item" data-testid={`context-item-${testKey}`}>
      <div className="context-bundle__item-head">
        <button
          type="button"
          className="linkbtn context-bundle__item-toggle"
          data-testid={`context-item-toggle-${testKey}`}
          aria-expanded={expanded}
          onClick={() => setExpanded((e) => !e)}
        >
          {expanded ? "▾" : "▸"}
        </button>
        {renaming ? (
          <span className="context-bundle__item-rename">
            <input
              className="builder-input"
              data-testid={`context-item-name-input-${testKey}`}
              value={nameDraft}
              aria-label="Item name"
              onChange={(e) => setNameDraft(e.target.value)}
            />
            <button
              type="button"
              className="linkbtn"
              data-testid={`context-item-rename-save-${testKey}`}
              disabled={rename.isPending}
              onClick={submitRename}
            >
              {rename.isPending ? "Saving…" : "Save"}
            </button>
            <button
              type="button"
              className="linkbtn"
              onClick={() => {
                setNameDraft(name);
                setRenaming(false);
              }}
            >
              Cancel
            </button>
          </span>
        ) : (
          <button
            type="button"
            className="context-bundle__item-name linkbtn"
            data-testid={`context-item-name-${testKey}`}
            title="Rename this item"
            onClick={() => {
              setNameDraft(name);
              setRenaming(true);
            }}
          >
            {name || "(unnamed)"}
          </button>
        )}
        {mediaType ? (
          <span className="context-bundle__item-type muted mono">{mediaType}</span>
        ) : null}
        <DigestChip hex={contentRef} label={name} />
        <button
          type="button"
          className="btn-ghost context-bundle__item-remove"
          data-testid={`context-item-remove-${testKey}`}
          disabled={remove.isPending || itemCount <= 1}
          title={
            itemCount <= 1
              ? "A bundle needs at least one item — delete the whole bundle instead"
              : "Remove this item from the bundle"
          }
          onClick={() => remove.mutate({ handle, itemIndex: index, expectBundleRef: bundleRef })}
        >
          {remove.isPending ? "Removing…" : "Remove"}
        </button>
      </div>

      {mutationError ? (
        <p className="field-error" data-testid={`context-item-error-${testKey}`} role="alert">
          {toUiError(mutationError).message}
        </p>
      ) : null}

      {expanded ? (
        <div className="context-bundle__item-body" data-testid={`context-item-body-${testKey}`}>
          {body.isLoading ? (
            <EmptyState title="Loading…" />
          ) : body.isError ? (
            <ErrorNotice error={toUiError(body.error)} onRetry={() => void body.refetch()} />
          ) : editing && decoded ? (
            <div className="context-bundle__item-edit">
              <TextEditor
                value={draft}
                language={editorLanguage(decoded.kind)}
                onChange={setDraft}
                testId={`context-item-editor-${testKey}`}
                ariaLabel="Edit item body"
              />
              <div className="context-bundle__item-actions">
                <button
                  type="button"
                  className="chip"
                  data-testid={`context-item-save-${testKey}`}
                  disabled={edit.isPending}
                  onClick={save}
                >
                  {edit.isPending ? "Saving…" : "Save"}
                </button>
                <button type="button" className="linkbtn" onClick={() => setEditing(false)}>
                  Cancel
                </button>
              </div>
            </div>
          ) : decoded ? (
            <div className="context-bundle__item-view">
              <AssetViewer
                content={decoded}
                stem={name || contentRef.slice(0, 8)}
                bodyTestId={`context-item-content-${testKey}`}
              />
              {editable ? (
                <button
                  type="button"
                  className="chip"
                  data-testid={`context-item-edit-${testKey}`}
                  onClick={startEditing}
                >
                  Edit
                </button>
              ) : (
                <p className="muted" data-testid={`context-item-readonly-${testKey}`}>
                  {decoded.truncated
                    ? "This item is too large to edit inline — download it, edit locally, and re-upload."
                    : "Binary / media items are download-only — edit locally and re-upload via the bundle form."}
                </p>
              )}
            </div>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}
