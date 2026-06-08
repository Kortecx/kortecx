import { useQueryClient } from "@tanstack/react-query";
import { Icon } from "./Icon";

/**
 * Global console controls. v1: refresh-all (invalidate every cached query so every
 * live view re-fetches at once). Filter/search controls land alongside ListRuns
 * (UI-2), once there is a cross-run dataset to filter.
 */
export function GlobalControls() {
  const qc = useQueryClient();
  return (
    <div className="controls" data-testid="global-controls">
      <button
        type="button"
        className="iconbtn"
        title="Refresh all data"
        aria-label="Refresh all data"
        data-testid="refresh-all"
        onClick={() => void qc.invalidateQueries()}
      >
        <Icon name="refresh" />
      </button>
    </div>
  );
}
