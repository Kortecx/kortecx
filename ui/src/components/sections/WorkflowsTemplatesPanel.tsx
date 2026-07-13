import { EmptyState } from "../EmptyState";

/**
 * The Templates tab — an honest placeholder for the reusable-templates feature
 * (start from a ready-made workflow, clone it, and enhance it with the model).
 * No fake controls (don't-fake-gaps): a clear "coming next" panel that marks the
 * seam the next increment fills.
 */
export function WorkflowsTemplatesPanel() {
  return (
    <div data-testid="workflows-templates">
      <EmptyState
        title="Reusable templates — coming next"
        detail="Start from a ready-made workflow template, clone it, and enhance it with the model and runtime — then run or schedule it like any other workflow."
        action={
          <span className="chip chip--soon" data-testid="workflows-templates-placeholder">
            Coming next
          </span>
        }
      />
    </div>
  );
}
