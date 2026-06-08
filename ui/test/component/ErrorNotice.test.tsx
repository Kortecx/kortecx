import {
  KxInvalidArgument,
  KxUnauthenticated,
  KxUnavailable,
  KxUnimplemented,
} from "@kortecx/sdk/web";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ErrorNotice } from "../../src/components/ErrorNotice";
import { toUiError } from "../../src/kx/errors";

describe("ErrorNotice", () => {
  it("reauth → a re-enter-token button that fires onReauth", () => {
    const onReauth = vi.fn();
    render(<ErrorNotice error={toUiError(new KxUnauthenticated("x"))} onReauth={onReauth} />);
    fireEvent.click(screen.getByRole("button", { name: /re-enter token/i }));
    expect(onReauth).toHaveBeenCalledOnce();
  });

  it("retryable → a retry button that fires onRetry", () => {
    const onRetry = vi.fn();
    render(<ErrorNotice error={toUiError(new KxUnavailable("x"))} onRetry={onRetry} />);
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("not-wired → no action buttons (retrying cannot help)", () => {
    render(
      <ErrorNotice
        error={toUiError(new KxUnimplemented("x"))}
        onRetry={vi.fn()}
        onReauth={vi.fn()}
      />,
    );
    expect(screen.getByTestId("error-notice")).toHaveAttribute("data-kind", "not-wired");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("bad-input → no retry button", () => {
    render(<ErrorNotice error={toUiError(new KxInvalidArgument("x"))} onRetry={vi.fn()} />);
    expect(screen.getByTestId("error-notice")).toHaveAttribute("data-kind", "bad-input");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
