"""Update workflow config (model, temperature, metadata)."""
import requests, json, sys

wf_id = sys.argv[1] if len(sys.argv) > 1 else None
if not wf_id:
    print("Usage: python update_config.py <workflow_id>"); exit(1)

updates = {
    "id": wf_id,
    "metadata": {
        "masterAgent": {"expertId": "local-project-orchestrator", "name": "Project Orchestrator", "role": "coordinator"},
        "inferenceConfig": {"kvCache": "auto", "memory": "standard", "quantization": "none"},
    },
}

resp = requests.patch("http://localhost:3000/api/workflows", json=updates)
print(f"✓ Updated" if resp.ok else f"✗ Failed: {resp.text[:200]}")
