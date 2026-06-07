import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ConnectionForm } from "../../src/components/ConnectionForm";

const LOOPBACK = "http://127.0.0.1:50151";
const REMOTE = "http://example.com:50151";

describe("ConnectionForm", () => {
  it("submits endpoint + token to onConnect (token stays in memory)", () => {
    const onConnect = vi.fn();
    render(<ConnectionForm initialEndpoint={LOOPBACK} connecting={false} onConnect={onConnect} />);
    fireEvent.change(screen.getByLabelText(/bearer token/i), { target: { value: "s3cr3t" } });
    fireEvent.click(screen.getByRole("button", { name: /^connect$/i }));
    expect(onConnect).toHaveBeenCalledWith(LOOPBACK, "s3cr3t");
  });

  it("a blank token connects with undefined", () => {
    const onConnect = vi.fn();
    render(<ConnectionForm initialEndpoint={LOOPBACK} connecting={false} onConnect={onConnect} />);
    fireEvent.click(screen.getByRole("button", { name: /^connect$/i }));
    expect(onConnect).toHaveBeenCalledWith(LOOPBACK, undefined);
  });

  it("warns when a token would cross plaintext http to a remote host", () => {
    render(<ConnectionForm initialEndpoint={REMOTE} connecting={false} onConnect={vi.fn()} />);
    expect(screen.queryByTestId("plaintext-warning")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/bearer token/i), { target: { value: "s3cr3t" } });
    expect(screen.getByTestId("plaintext-warning")).toBeInTheDocument();
  });

  it("does not warn for a loopback endpoint", () => {
    render(<ConnectionForm initialEndpoint={LOOPBACK} connecting={false} onConnect={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/bearer token/i), { target: { value: "s3cr3t" } });
    expect(screen.queryByTestId("plaintext-warning")).not.toBeInTheDocument();
  });

  it("disables connect on an invalid endpoint", () => {
    render(<ConnectionForm initialEndpoint="" connecting={false} onConnect={vi.fn()} />);
    expect(screen.getByRole("button", { name: /^connect$/i })).toBeDisabled();
  });
});
