"""Add steps to an existing workflow."""
import requests, json, sys

wf_id = sys.argv[1] if len(sys.argv) > 1 else None
if not wf_id:
    print("Usage: python add_steps.py <workflow_id>"); exit(1)

# Fetch existing workflow
wf = requests.get(f"http://localhost:3000/api/workflows?id={wf_id}").json().get("workflow", {})
if not wf:
    print(f"✗ Workflow {wf_id} not found"); exit(1)

# Add new step
new_step = {
    "order": len(wf.get("steps", [])) + 1,
    "name": "Additional Review Step",
    "taskDescription": "Perform a final quality review of all outputs.",
    "modelSource": "local",
    "localModelConfig": {"engine": "ollama", "modelName": "llama3.2:3b"},
    "connectionType": "sequential", "stepType": "agent",
    "temperature": 0.3, "maxTokens": 2048,
}

existing_steps = wf.get("steps", [])
existing_steps.append(new_step)

resp = requests.patch(f"http://localhost:3000/api/workflows", json={"id": wf_id, "steps": existing_steps})
print(f"✓ Step added, total steps: {len(existing_steps)}" if resp.ok else f"✗ Failed: {resp.text[:200]}")
