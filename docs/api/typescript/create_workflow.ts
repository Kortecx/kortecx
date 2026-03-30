/**
 * Create a workflow with steps (TypeScript)
 * Run: npx tsx docs/api/typescript/create_workflow.ts
 */
async function createWorkflow() {
  const body = {
    name: process.argv[2] || "TS Pipeline",
    description: "TypeScript-created workflow",
    goalStatement: "Generate and review a REST API.",
    status: "ready",
    tags: ["ts-api"],
    steps: [
      {
        order: 1, name: "Generate",
        taskDescription: "Generate FastAPI endpoints.",
        modelSource: "local",
        localModelConfig: { engine: "ollama", modelName: "llama3.2:3b" },
        connectionType: "sequential", stepType: "agent",
        temperature: 0.7, maxTokens: 4096,
      },
    ],
    metadata: { masterAgent: null, graphNodes: [], graphEdges: [], nodeConfigs: {} },
  };

  const resp = await fetch("http://localhost:3000/api/workflows", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const data = await resp.json();
  console.log(resp.ok ? `✓ Workflow: ${data.workflow?.id}` : `✗ ${data.error}`);
}

createWorkflow().catch(console.error);
