import type { UiError } from "../kx/errors";

export interface ErrorNoticeProps {
  error: UiError;
  onRetry?: () => void;
  onReauth?: () => void;
}

/** Render a {@link UiError} with the affordance its kind implies. */
export function ErrorNotice({ error, onRetry, onReauth }: ErrorNoticeProps) {
  return (
    <div
      className={`notice notice--${error.kind}`}
      role="alert"
      data-testid="error-notice"
      data-kind={error.kind}
    >
      <div className="notice__head">
        <strong className="notice__title">{error.title}</strong>
        <code className="notice__code">[{error.code}]</code>
      </div>
      <p className="notice__message">{error.message}</p>
      <div className="notice__actions">
        {error.kind === "reauth" && onReauth ? (
          <button type="button" onClick={onReauth}>
            Re-enter token
          </button>
        ) : null}
        {error.retryable && onRetry ? (
          <button type="button" onClick={onRetry}>
            Retry
          </button>
        ) : null}
      </div>
    </div>
  );
}
