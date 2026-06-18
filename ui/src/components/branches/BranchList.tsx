/**
 * The D155 branch inventory (`ListBranches`) — the GOVERNANCE + review view:
 * every branch this party authored, its `{path → ref}` manifest (each ref shown
 * via {@link DigestChip}), the server-derived `branchRef`, the CoW parent (if a
 * fork), and an operator delete control (unbinds the handle; the CAS blobs stay).
 * Caller-scoped (SN-8). Degrades to a not-wired empty state on an older gateway.
 */

import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useBranches, useDeleteBranch, useEditBranch } from "../../kx/use-branches";
import { DigestChip } from "../DigestChip";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

export function BranchList() {
  const { branches, notWired, isLoading, isError, error, refetch } = useBranches();
  const remove = useDeleteBranch();
  const removeError = remove.error ? toUiError(remove.error) : null;

  if (isLoading) {
    return <EmptyState title="Loading branches…" />;
  }
  if (notWired) {
    return (
      <EmptyState
        title="Branches need a newer gateway"
        detail="This gateway doesn't expose the branch store (an older build)."
      />
    );
  }
  if (isError) {
    return <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />;
  }

  return (
    <div data-testid="branches">
      {removeError ? (
        <p className="field-error" data-testid="branch-delete-error" role="alert">
          {removeError.message}
        </p>
      ) : null}
      {branches.length === 0 ? (
        <EmptyState
          title="No branches yet"
          detail="Snapshot a path set below to create one — the operator must run kx serve with KX_SERVE_FS_ROOT set."
        />
      ) : (
        <m.ul
          className="registry-list"
          data-testid="branches-panel"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {branches.map((b) => {
            const pending = remove.isPending && remove.variables?.handle === b.handle;
            return (
              <BranchRow
                key={b.handle}
                handle={b.handle}
                branchRef={b.branchRef}
                parentHandle={b.parentHandle}
                description={b.description}
                itemCount={b.itemCount}
                items={b.items}
                pending={pending}
                onDelete={() => remove.mutate({ handle: b.handle })}
              />
            );
          })}
        </m.ul>
      )}
    </div>
  );
}

interface BranchItemView {
  readonly path: string;
  readonly contentRef: string;
}

function BranchRow({
  handle,
  branchRef,
  parentHandle,
  description,
  itemCount,
  items,
  pending,
  onDelete,
}: {
  handle: string;
  branchRef: string;
  parentHandle: string;
  description: string;
  itemCount: number;
  items: readonly BranchItemView[];
  pending: boolean;
  onDelete: () => void;
}) {
  const edit = useEditBranch();
  const [editingPath, setEditingPath] = useState<string | null>(null);
  const [instruction, setInstruction] = useState("");
  const editError = edit.error ? toUiError(edit.error) : null;

  const submitEdit = (path: string) => {
    const trimmed = instruction.trim();
    if (trimmed.length === 0) {
      return;
    }
    edit.mutate(
      { handle, path, instruction: trimmed },
      {
        onSuccess: () => {
          setEditingPath(null);
          setInstruction("");
        },
      },
    );
  };

  return (
    <GlowCard
      className="registry-row"
      stripe="var(--teal)"
      variants={fadeUp}
      data-testid={`branch-${handle}`}
      {...hoverLift}
    >
      <div className="registry-row__main">
        <div className="registry-row__head">
          <span className="registry-row__name mono">{handle}</span>
          <Badge label={`${itemCount} file${itemCount === 1 ? "" : "s"}`} color="var(--teal)" />
          {parentHandle ? <Badge label={`← ${parentHandle}`} color="var(--violet)" /> : null}
          <DigestChip hex={branchRef} label={`${handle} branch ref`} />
        </div>
        {description ? <p className="registry-row__desc muted">{description}</p> : null}
        {items.length > 0 ? (
          <ul className="context-bundle__items">
            {items.map((it) => {
              const isEditing = editingPath === it.path;
              const editingThis = edit.isPending && edit.variables?.path === it.path;
              return (
                <li key={it.path} className="context-bundle__item branch-item">
                  <span className="context-bundle__item-name mono">{it.path}</span>
                  <DigestChip hex={it.contentRef} label={it.path} />
                  <button
                    type="button"
                    className="btn-ghost branch-item__edit"
                    data-testid={`branch-edit-${handle}-${it.path}`}
                    disabled={edit.isPending}
                    title="Rewrite this file in-CAS via the agent (the host is never written)"
                    onClick={() => {
                      setEditingPath(isEditing ? null : it.path);
                      setInstruction("");
                    }}
                  >
                    {isEditing ? "Cancel" : "Edit"}
                  </button>
                  {isEditing ? (
                    <div className="branch-item__editor" data-testid="branch-edit-form">
                      <textarea
                        className="input"
                        data-testid="branch-edit-instruction"
                        placeholder="Describe the change (e.g. 'uppercase the title line')…"
                        rows={2}
                        value={instruction}
                        disabled={editingThis}
                        onChange={(e) => setInstruction(e.target.value)}
                      />
                      <button
                        type="button"
                        className="btn-primary"
                        data-testid="branch-edit-submit"
                        disabled={editingThis || instruction.trim().length === 0}
                        onClick={() => submitEdit(it.path)}
                      >
                        {editingThis ? "Editing…" : "Run edit"}
                      </button>
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        ) : null}
        {editError ? (
          <p className="field-error" data-testid="branch-edit-error" role="alert">
            {editError.message}
          </p>
        ) : null}
      </div>
      <button
        type="button"
        className="btn-ghost registry-row__deregister"
        data-testid={`branch-delete-${handle}`}
        disabled={pending}
        title="Unbind this branch (its content-store blobs stay)"
        onClick={onDelete}
      >
        {pending ? "Removing…" : "Delete"}
      </button>
    </GlowCard>
  );
}
