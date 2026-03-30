"""Attach a plugin to a workflow step."""
import requests, json, sys

wf_id = sys.argv[1] if len(sys.argv) > 1 else None
plugin_id = sys.argv[2] if len(sys.argv) > 2 else "web-scraper"

if not wf_id:
    print("Usage: python connect_plugin.py <workflow_id> [plugin_id]"); exit(1)

# Add plugin step to workflow
new_step = {
    "order": 99,  # Will be re-ordered
    "name": f"Plugin: {plugin_id}",
    "stepType": "action",
    "actionConfig": {"transformerType": "mcp", "mcpServerId": plugin_id, "outputFormat": "markdown"},
    "taskDescription": f"Execute {plugin_id} plugin.",
    "modelSource": "local",
    "localModelConfig": {"engine": "ollama", "modelName": "llama3.2:3b"},
    "connectionType": "sequential",
    "temperature": 0.7, "maxTokens": 2048,
}

wf = requests.get(f"http://localhost:3000/api/workflows?id={wf_id}").json().get("workflow", {})
steps = wf.get("steps", [])
new_step["order"] = len(steps) + 1
steps.append(new_step)

resp = requests.patch("http://localhost:3000/api/workflows", json={"id": wf_id, "steps": steps})
print(f"✓ Plugin '{plugin_id}' attached as step {new_step['order']}" if resp.ok else f"✗ Failed")
