/**
 * Execute a workflow via the engine (TypeScript)
 * Run: npx tsx docs/api/typescript/run_workflow.ts
 */
const ENGINE = "http://localhost:8000";

async function runWorkflow() {
  const resp = await fetch(`${ENGINE}/api/orchestrator/execute`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      name: "TS Test Run",
      workflowId: `wf-ts-${Date.now()}`,
      goalFileUrl: "",
      steps: [{
        stepId: "s1", name: "Execute",
        taskDescription: process.argv[2] || "Write a hello world Python script.",
        modelSource: "local",
        localModel: { engine: "ollama", model: "llama3.2:3b" },
        temperature: 0.7, maxTokens: 1024, connectionType: "sequential",
      }],
    }),
  });
  const data = await resp.json();
  console.log(`✓ Run: ${data.runId}, Status: ${data.status}`);

  // Poll
  for (let i = 0; i < 20; i++) {
    await new Promise(r => setTimeout(r, 5000));
    const st = await fetch(`${ENGINE}/api/orchestrator/status`).then(r => r.json());
    console.log(`  [${(i+1)*5}s] active=${st.active_runs}`);
    if (st.active_runs === 0) { console.log("✓ Done"); break; }
  }
}

runWorkflow().catch(console.error);
