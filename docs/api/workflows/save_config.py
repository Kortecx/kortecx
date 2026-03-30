"""Save workflow config locally with versioning."""
import requests, json, sys

wf_name = sys.argv[1] if len(sys.argv) > 1 else "test-workflow"
config = {"name": wf_name, "description": "Test config save", "steps": [], "tags": []}

resp = requests.post("http://localhost:3000/api/workflows/save-local", json={
    "workflowName": wf_name, "config": config, "graph": {"nodes": [], "edges": []}, "maxVersions": 3,
})
print(f"✓ Saved: {resp.json()}" if resp.ok else f"✗ Failed: {resp.text}")
