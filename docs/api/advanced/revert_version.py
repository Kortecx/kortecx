"""Revert to a previous workflow config version."""
import requests, json, sys

wf_name = sys.argv[1] if len(sys.argv) > 1 else None
timestamp = sys.argv[2] if len(sys.argv) > 2 else None

if not wf_name or not timestamp:
    print("Usage: python revert_version.py <workflow_name> <timestamp>")
    print("  Get timestamps from: python list_versions.py <workflow_name>"); exit(1)

resp = requests.get(f"http://localhost:8000/api/orchestrator/workflow-versions/{wf_name}/{timestamp}")
data = resp.json()
config = data.get("config")
if config:
    print(f"✓ Loaded version {timestamp}:")
    print(f"  Name: {config.get('name')}, Steps: {len(config.get('steps', []))}")
    print(f"  Apply via: PATCH /api/workflows with this config")
else:
    print(f"✗ Version not found: {data.get('error')}")
