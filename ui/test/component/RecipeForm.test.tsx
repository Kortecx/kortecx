import { RecipeForm as RecipeFormDef, RecipeFormField } from "@kortecx/sdk/web";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RecipeForm } from "../../src/components/recipes/RecipeForm";

describe("RecipeForm", () => {
  it("renders an input per free-param and submits coerced args", () => {
    const form = new RecipeFormDef("kx/recipes/x", [
      new RecipeFormField("topic", "str", true, 4096, []),
      new RecipeFormField("count", "int", true, null, []),
    ]);
    const onSubmit = vi.fn();
    render(<RecipeForm form={form} pending={false} onSubmit={onSubmit} />);

    fireEvent.change(screen.getByTestId("field-topic"), { target: { value: "hello" } });
    fireEvent.change(screen.getByTestId("field-count"), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: /run recipe/i }));

    expect(onSubmit).toHaveBeenCalledWith({ topic: "hello", count: 3 });
  });

  it("renders an enum as a select and submits the chosen value", () => {
    const form = new RecipeFormDef("kx/recipes/x", [
      new RecipeFormField("mode", "enum", true, null, ["fast", "slow"]),
    ]);
    const onSubmit = vi.fn();
    render(<RecipeForm form={form} pending={false} onSubmit={onSubmit} />);
    fireEvent.change(screen.getByTestId("field-mode"), { target: { value: "slow" } });
    fireEvent.click(screen.getByRole("button", { name: /run recipe/i }));
    expect(onSubmit).toHaveBeenCalledWith({ mode: "slow" });
  });

  it("shows a validation error and does not submit a bad value", () => {
    const form = new RecipeFormDef("kx/recipes/x", [
      new RecipeFormField("topic", "str", true, null, []),
    ]);
    const onSubmit = vi.fn();
    render(<RecipeForm form={form} pending={false} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole("button", { name: /run recipe/i }));
    expect(screen.getByRole("alert")).toHaveTextContent("required");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("a no-input recipe runs directly with empty args", () => {
    const form = new RecipeFormDef("kx/recipes/fanout-demo", []);
    const onSubmit = vi.fn();
    render(<RecipeForm form={form} pending={false} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole("button", { name: /run recipe/i }));
    expect(onSubmit).toHaveBeenCalledWith({});
  });
});
