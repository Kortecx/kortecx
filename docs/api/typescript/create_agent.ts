/**
 * Create an agent via the Kortecx API (TypeScript)
 * Run: npx tsx docs/api/typescript/create_agent.ts
 */
const FRONTEND = "http://localhost:3000";

async function createAgent() {
  const agent = {
    name: process.argv[2] || "TS Sample Agent",
    role: "coder",
    description: "TypeScript-based agent for code generation.",
    systemPrompt: "You are an expert TypeScript developer.",
    modelSource: "local",
    localModelConfig: { engine: "ollama", modelName: "llama3.2:3b" },
    temperature: 0.7,
    maxTokens: 4096,
    tags: ["typescript", "code-gen"],
    capabilities: ["code-generation"],
    category: "custom",
    complexityLevel: 3,
  };

  const resp = await fetch(`${FRONTEND}/api/experts`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(agent),
  });
  const data = await resp.json();
  const e = data.expert || {};
  console.log(resp.ok ? `✓ Agent: ${e.id}` : `✗ Failed: ${data.error}`);
}

createAgent().catch(console.error);
