"""Delete a workflow run record."""
import requests, sys

run_id = sys.argv[1] if len(sys.argv) > 1 else None
if not run_id:
    print("Usage: python delete_run.py <run_id>"); exit(1)

resp = requests.delete(f"http://localhost:3000/api/workflows/runs?id={run_id}")
print(f"✓ Deleted" if resp.ok else f"✗ Failed: {resp.json().get('error')}")
