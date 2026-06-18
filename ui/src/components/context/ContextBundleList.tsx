/**
 * The context-bundle inventory (`ListContextBundles`, PR-7) — the GOVERNANCE +
 * review view: every bundle this party authored, its items (each a content-store
 * ref shown via {@link DigestChip}), the server-derived `bundleRef`, and an
 * operator delete control (unbinds the handle; the CAS blobs stay). Caller-scoped
 * (SN-8 — no cross-party listing). Degrades to a not-wired empty state on an older
 * gateway (UNIMPLEMENTED).
 */

import { m } from "framer-motion";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useContextBundles, useDeleteContextBundle } from "../../kx/use-context-bundles";
import { DigestChip } from "../DigestChip";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

export function ContextBundleList() {
  const { bundles, notWired, isLoading, isError, error, refetch } = useContextBundles();
  const remove = useDeleteContextBundle();
  const removeError = remove.error ? toUiError(remove.error) : null;

  if (isLoading) {
    return <EmptyState title="Loading context bundles…" />;
  }
  if (notWired) {
    return (
      <EmptyState
        title="Context bundles need a newer gateway"
        detail="This gateway doesn't expose the context-bundle store (an older build)."
      />
    );
  }
  if (isError) {
    return <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />;
  }

  return (
    <div data-testid="context-bundles">
      {removeError ? (
        <p className="field-error" data-testid="context-bundle-delete-error" role="alert">
          {removeError.message}
        </p>
      ) : null}
      {bundles.length === 0 ? (
        <EmptyState
          title="No context bundles yet"
          detail="Author one below — attach files or content refs under a handle, then attach it to a chat or chain."
        />
      ) : (
        <m.ul
          className="registry-list"
          data-testid="context-bundles-panel"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {bundles.map((b) => {
            const pending = remove.isPending && remove.variables?.handle === b.handle;
            return (
              <BundleRow
                key={b.handle}
                handle={b.handle}
                bundleRef={b.bundleRef}
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

interface BundleItemView {
  readonly name: string;
  readonly contentRef: string;
  readonly mediaType: string;
}

function BundleRow({
  handle,
  bundleRef,
  description,
  itemCount,
  items,
  pending,
  onDelete,
}: {
  handle: string;
  bundleRef: string;
  description: string;
  itemCount: number;
  items: readonly BundleItemView[];
  pending: boolean;
  onDelete: () => void;
}) {
  return (
    <GlowCard
      className="registry-row"
      stripe="var(--teal)"
      variants={fadeUp}
      data-testid={`context-bundle-${handle}`}
      {...hoverLift}
    >
      <div className="registry-row__main">
        <div className="registry-row__head">
          <span className="registry-row__name mono">{handle}</span>
          <Badge label={`${itemCount} item${itemCount === 1 ? "" : "s"}`} color="var(--teal)" />
          <DigestChip hex={bundleRef} label={`${handle} bundle ref`} />
        </div>
        {description ? <p className="registry-row__desc muted">{description}</p> : null}
        {items.length > 0 ? (
          <ul className="context-bundle__items">
            {items.map((it) => (
              <li key={`${it.name}:${it.contentRef}`} className="context-bundle__item">
                <span className="context-bundle__item-name">{it.name || "(unnamed)"}</span>
                {it.mediaType ? (
                  <span className="context-bundle__item-type muted mono">{it.mediaType}</span>
                ) : null}
                <DigestChip hex={it.contentRef} label={it.name} />
              </li>
            ))}
          </ul>
        ) : null}
      </div>
      <button
        type="button"
        className="btn-ghost registry-row__deregister"
        data-testid={`context-bundle-delete-${handle}`}
        disabled={pending}
        title="Unbind this bundle (its content-store blobs stay)"
        onClick={onDelete}
      >
        {pending ? "Removing…" : "Delete"}
      </button>
    </GlowCard>
  );
}
