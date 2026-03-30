"""Create a workflow with steps."""
import requests, json, sys

name = sys.argv[1] if len(sys.argv) > 1 else "API Pipeline"

body = {
    "name": name,
    "description": "Multi-step API generation workflow",
    "goalStatement": "Generate a REST API with CRUD endpoints, review it, and produce documentation.",
    "status": "ready",
    "tags": ["api", "python"],
    "steps": [
        {
            "order": 1, "name": "Generate Code",
            "taskDescription": "Generate FastAPI CRUD endpoints with Pydantic models.",
            "modelSource": "local",
            "localModelConfig": {"engine": "ollama", "modelName": "llama3.2:3b"},
            "connectionType": "sequential", "stepType": "agent",
            "temperature": 0.7, "maxTokens": 4096,
        },
        {
            "order": 2, "name": "Review Code",
            "taskDescription": "Review the generated code for security and best practices.",
            "modelSource": "local",
            "localModelConfig": {"engine": "ollama", "modelName": "llama3.2:3b"},
            "connectionType": "sequential", "stepType": "agent",
            "temperature": 0.3, "maxTokens": 2048,
        },
    ],
    "metadata": {"masterAgent": None, "graphNodes": [], "graphEdges": [], "nodeConfigs": {}},
}

resp = requests.post("http://localhost:3000/api/workflows", json=body)
data = resp.json()
w = data.get("workflow", {})
print(f"✓ Workflow: {w.get('id')}, Status: {w.get('status')}" if resp.ok else f"✗ Failed: {data.get('error')}")
