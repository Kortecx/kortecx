import { REACT_AUTO_RECIPE_HANDLE } from "../../kx/use-chat";
import { useRecipes } from "../../kx/use-recipes";

/**
 * PR-6b-4 — the honest auto-grant status row. When the operator runs with
 * `KX_SERVE_AUTOGRANT`, the runtime seeds `kx/recipes/react-auto` (a live ReAct
 * loop that auto-grants the registered/dialed tool set, capped). Its presence in
 * `ListRecipes` is the source of truth, so this row reflects the REAL serve
 * posture — never a faked control (SN-8 / GR15): OSS exposes no toggle; enabling
 * it is an operator/Cloud concern. Absent / loading / unwired all read OFF (the
 * recipe genuinely isn't live), which is the honest default-OFF.
 */
export function AutoGrantStatus() {
  const recipes = useRecipes();
  const on = (recipes.data ?? []).includes(REACT_AUTO_RECIPE_HANDLE);

  return (
    <div className="autogrant-status" data-testid="autogrant-status">
      <span
        className={`status-dot ${on ? "status-dot--online" : "status-dot--offline"}`}
        aria-hidden="true"
      />
      <span className="autogrant-status__label">Auto-grant</span>
      <span
        className={`pill ${on ? "pill--committed" : "pill--unknown"}`}
        data-testid="autogrant-pill"
      >
        {on ? "ON" : "OFF"}
      </span>
      <span className="muted autogrant-status__detail">
        {on
          ? "kx/recipes/react-auto is live — the autonomous loop may fire any registered or dialed tool (capped, re-verified per call)."
          : "Run with KX_SERVE_AUTOGRANT to let the autonomous loop auto-grant the registered/dialed tool set (kx/recipes/react-auto)."}
      </span>
    </div>
  );
}
