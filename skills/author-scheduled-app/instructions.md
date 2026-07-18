# Authoring a scheduled app

A **scheduled app** is a durable, reusable automation: a blueprint the runtime
runs **unattended** on a trigger (a cron schedule, a webhook, or a gRPC call) and
that can be composed into workflows. Author it comprehensively — a good scheduled
app does an end-to-end job, not a single step.

## Procedure

1. **State the job and its trigger.** Decide *when* it runs — a cron spec (e.g.
   `0 9 * * 1-5`), a webhook, or on-demand. The app runs with **no human present**,
   so every step must be self-contained.

2. **Break the goal into a small pipeline.** Use focused roles (researcher /
   analyst / writer) as steps. For each step, name only the tools it needs from the
   granted palette (search / read / retrieve, and *draft* / *create* where a
   deliverable is produced).

3. **Wire connections + integrations.** For each external service the job touches
   (Gmail, Notion, Slack, Discord), declare the connection by descriptor and
   credential **name** — the runtime resolves the secret at dial time; the app never
   carries it.

4. **Ground it.** Attach `retrieve@1` grounding over the relevant datasets/context
   so the run reasons over real material, not guesses.

5. **Respect the boundaries (honesty).**
   - **Never** wish for an irreversible send (`gmail/send`, `slack/post_message`,
     `discord/send_message`, `notion/append_block`). An unattended run must **stage**
     an irreversible action for human approval (HITL), never fire it silently.
   - Only claim tools the run will actually be granted. If a needed connection is
     absent, say so — do not fabricate a capability.

6. **Output contract.** A saved `kortecx.app/v1` envelope plus a registered trigger,
   ready to run on schedule or plug into a workflow.
