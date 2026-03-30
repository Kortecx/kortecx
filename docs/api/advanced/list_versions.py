"""List workflow config versions."""
import requests, json, sys

wf_name = sys.argv[1] if len(sys.argv) > 1 else "test-workflow"
resp = requests.get(f"http://localhost:8000/api/orchestrator/workflow-versions/{wf_name}")
data = resp.json()
versions = data.get("versions", [])
print(f"Versions for '{wf_name}': {len(versions)}\n")
for v in versions:
    print(f"  ts={v['timestamp']}  date={v['date']}  files={v['files']}")
