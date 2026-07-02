import type { UiError } from "../kx/errors";

export interface ErrorNoticeProps {
  error: UiError;
  onRetry?: () => void;
  onReauth?: () => void;
  /** An optional caller-supplied remediation (e.g. G2 "Set up integration" → open the
   * Connections panel). Rendered as a primary action button alongside Retry/Re-auth. */
  action?: { label: string; onClick: () => void };
}

/** Render a {@link UiError} with the affordance its kind implies. */
export function ErrorNotice({ error, onRetry, onReauth, action }: ErrorNoticeProps) {
  return (
    <div
      className={`notice notice--${error.kind}`}
      role="alert"
      data-testid="error-notice"
      data-kind={error.kind}
    >
      <div className="notice__head">
        <strong className="notice__title">{error.title}</strong>
        {error.refusalCode ? (
          <code
            className="notice__code"
            data-testid="refusal-code"
            title="The runtime's structured refusal code"
          >
            refusal {error.refusalCode}
          </code>
        ) : null}
        <code className="notice__code">[{error.code}]</code>
      </div>
      <p className="notice__message">{error.message}</p>
      <div className="notice__actions">
        {action ? (
          <button type="button" data-testid="error-notice-action" onClick={action.onClick}>
            {action.label}
          </button>
        ) : null}
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
