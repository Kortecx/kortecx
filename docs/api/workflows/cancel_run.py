"""Cancel a running workflow."""
import requests, sys

run_id = sys.argv[1] if len(sys.argv) > 1 else None
wf_id = sys.argv[2] if len(sys.argv) > 2 else None
if not run_id:
    print("Usage: python cancel_run.py <run_id> [workflow_id]"); exit(1)

resp = requests.post("http://localhost:3000/api/workflows/stop", json={"runId": run_id, "workflowId": wf_id})
print(f"✓ Cancelled: {resp.json()}" if resp.ok else f"✗ Failed: {resp.json().get('error')}")
