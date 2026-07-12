/**
 * PR-D: classify a run Mote into a HIGH-LEVEL step type for the read-only run
 * review — the human-legible "what did this step do" over the runtime's
 * `WorkflowStepKind` (PURE/MODEL/EXEC/TOOL) + the step's tool contract. Pure +
 * total (a bad/absent kind falls to `unknown`), so it is unit-tested directly and
 * carries no authority (SN-8 — display only).
 */

/** The high-level step type shown as a node badge on the review DAG. */
export type StepType = "model" | "mcp" | "connector" | "tool" | "action" | "unknown";

/** Display label per step type. */
export const STEP_LABEL: Record<StepType, string> = {
  model: "Model",
  mcp: "MCP",
  connector: "Connector",
  tool: "Tool",
  action: "Action",
  unknown: "Step",
};

/** Community connectors ship as backend sidecars; a tool whose name names one is a
 *  connector rather than a generic tool (display heuristic only). */
const CONNECTOR_RE = /\b(gmail|slack|notion|discord|connector)\b/;

/**
 * Classify from the admitted step kind + its tool contract (both from
 * `GetMoteDetail`). A TOOL step is refined by its tool name: an MCP tool (dialed
 * through the egress gateway, keyed `server/tool`) vs a named community connector vs
 * a generic registered tool. Robust to either the raw enum or a friendly kind string.
 */
export function classifyStep(
  stepKind: string,
  toolContract: Readonly<Record<string, string>> = {},
): StepType {
  const kind = stepKind.toUpperCase();
  const tools = Object.keys(toolContract);
  if (kind.includes("MODEL")) {
    return "model";
  }
  if (kind.includes("TOOL") || tools.length > 0) {
    const name = (tools[0] ?? "").toLowerCase();
    if (name.startsWith("mcp") || name.includes("/")) {
      return "mcp";
    }
    if (CONNECTOR_RE.test(name)) {
      return "connector";
    }
    return "tool";
  }
  if (kind.includes("PURE") || kind.includes("EXEC")) {
    return "action";
  }
  return "unknown";
}
