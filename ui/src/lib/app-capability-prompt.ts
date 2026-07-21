/**
 * PR-G: the client-composed CAPABILITY PROMPT baked into a new App's envelope (as a
 * `references.rules` guidance item). It grounds the App's chosen model in what a
 * Kortecx App is and how to drive one through the runtime — so the model can
 * orchestrate the goal end-to-end using the tools, connections, datasets, skills,
 * and attached context the runtime exposes. Functional guidance only (no strategy):
 * it describes the runtime capabilities the App actually has — read-only retrieval,
 * no writable project branch — and insists on honesty (never invent a tool, a
 * result, or a capability the App was not given).
 */

/** The base capability instruction (goal-independent). Composed with the App's own
 *  goal + any attached context at authoring time. */
export const CAPABILITY_PROMPT = `You are the agent that powers a Kortecx App — a durable, portable, agentic blueprint that carries a user's goal to completion by reasoning and acting through the Kortecx runtime.

How you work (the reason → act → observe loop):
- Reason about the goal and the current state.
- Act: call a tool, retrieve from a dataset, read an attached context file, or use a connected integration.
- Observe the result, then decide the next action. Repeat until the goal is fully done, then produce the final result as your answer.

What the runtime gives this App:
- Tools — the registered tools available to this App, each with a pinned contract. Call a tool by its exact name; the runtime validates and runs it.
- Connections & integrations — connected external services (e.g. MCP servers, connectors) the App may use through the runtime's secure gateway to fetch or send information.
- Datasets (RAG) — the App's grounding corpora. Use the retrieve tool to search a dataset and ground your answer in the retrieved passages; cite what you used.
- Skills — reusable instruction + tool bundles attached to the App; follow their guidance when relevant.
- Context files & attachments — files attached to the App (plans, specs, examples). Read them for context before acting.
- Retrieval is read-only. Your access to the App's datasets and attached context is READ-ONLY (via the retrieve tool). This App has no writable project branch or file system — you cannot create, edit, or save files; return your work in your answer instead.

How to structure the work:
- Break the goal into small, concrete, verifiable steps.
- Produce the App's deliverable (the summary, analysis, code, or data the goal asks for) INLINE as your final answer — you cannot write it to files, so return it in full in your response.
- Use each tool/integration by its exact contract; do not invent a capability that is not offered.
- If several agents or a swarm collaborate, each records its result and reasoning so the next step can summarize, verify, and continue.
- Keep going until the goal is fully accomplished, then present the complete final output in your answer.

Honesty:
- Use only the tools, connections, and datasets actually available to this App. If something needed is missing, say so plainly.
- Never fabricate a tool result, a citation, or a capability you were not given.`;

/** Which lane the App is authored for — steers the capability guidance (D213). */
export type AppPromptKind = "agent" | "scheduled" | "hosted";

/** Appended for a SCHEDULED (functional) App — it may run unattended on a trigger, so
 *  irreversible actions are STAGED for approval, never fired silently. */
const SCHEDULED_NOTE = `
This App may run UNATTENDED on a trigger (a cron schedule, a webhook, or a gRPC call) — no human is present at run time. Use the connected integrations and tools it was granted to carry the goal end-to-end and return the result. Any irreversible action (sending mail, posting to a channel, writing to an external system) is STAGED for human approval, not fired — do not assume it will send. This App has no writable project branch; return your work in your answer.`;

/** The HOSTED (experience) App authoring prompt — a real web project, not an agent loop.
 *  Honest by construction: a local dev server on a local port, no baked live-data access. */
const HOSTED_PROMPT = `You are authoring a HOSTED Kortecx App — a real web application (a Vite-React or Next.js project) that the runtime scaffolds into a project tree and serves on a LOCAL port.

What to produce:
- A single, self-contained page that implements exactly what the user described, using ONLY React (and Next.js for a Next app) — no extra npm dependencies. Make it render immediately.
- Clean, working code; prefer inline styles or the project's stylesheet over new dependencies.

Boundaries (honesty):
- This app runs as a LOCAL web app on a local port — never a public URL (that is a Cloud capability).
- It does NOT have baked access to the user's live data, the internet, or the runtime's tools. A hosted app reaches runtime capabilities only through the governed request seam, and only when that is wired. Do not claim live data or actions it cannot perform.`;

/**
 * Compose the full capability guidance for an App from the base prompt + the App's
 * goal + optional attachment filenames (so the model knows what was attached). `kind`
 * selects the lane-appropriate guidance; the default (`"agent"`) is unchanged.
 */
export function composeCapabilityPrompt(
  goal: string,
  attachments: readonly string[] = [],
  kind: AppPromptKind = "agent",
): string {
  const g = goal.trim();
  const parts = [kind === "hosted" ? HOSTED_PROMPT : CAPABILITY_PROMPT];
  if (kind === "scheduled") {
    parts.push(SCHEDULED_NOTE);
  }
  if (g !== "") {
    parts.push(`\nThis App's goal:\n${g}`);
  }
  if (attachments.length > 0) {
    parts.push(`\nAttached context files: ${attachments.join(", ")}. Read them for context.`);
  }
  return parts.join("\n");
}

/**
 * Compose everything the author has said about the App into the ONE string
 * `ProposeWorkflow` accepts, so the planner sees the whole brief rather than a fragment.
 *
 * The propose button used to send `goal` alone. The name, the prompt (the instruction
 * the App actually runs each time) and the attached files were all on screen and none of
 * them reached the planner — which is why a proposed plan could look unrelated to what
 * the author had just typed.
 *
 * `ProposeWorkflowRequest` has exactly one field and the server interpolates it verbatim
 * into the plan prompt, so composing here needs no wire change. Labels are plain and
 * stable; the model reads this as prose, not as a schema.
 *
 * ATTACHMENTS ARE NAMED, NEVER REFERENCED. Filenames tell the planner what material
 * exists; the content refs deliberately stay out, because the planner has no grant to
 * dereference them and handing it an identifier it cannot resolve invites a plan that
 * pretends to have read the file.
 */
export function composeProposeGoal(input: {
  name: string;
  goal: string;
  prompt?: string;
  attachments?: readonly string[];
}): string {
  const name = input.name.trim();
  const goal = input.goal.trim();
  const prompt = (input.prompt ?? "").trim();
  const files = (input.attachments ?? []).filter((f) => f.trim() !== "");
  // A bare goal composes to itself — so the common case is byte-identical to what the
  // planner received before, and nothing about its behaviour changes for it.
  if (name === "" && prompt === "" && files.length === 0) {
    return goal;
  }
  const parts: string[] = [];
  if (name !== "") {
    parts.push(`App: ${name}`);
  }
  parts.push(`Goal: ${goal}`);
  if (prompt !== "") {
    parts.push(`Instruction it runs each time: ${prompt}`);
  }
  if (files.length > 0) {
    parts.push(`Context files it can read: ${files.join(", ")}`);
  }
  return parts.join("\n");
}
