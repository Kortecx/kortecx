"""List run output files."""
import requests, json, sys

wf_name = sys.argv[1] if len(sys.argv) > 1 else "api-design-pipeline"
resp = requests.get(f"http://localhost:3000/api/workflows/outputs?workflowName={wf_name}")
runs = resp.json().get("runs", [])
print(f"Output runs: {len(runs)}\n")
for r in runs[:5]:
    print(f"  Run: {r['runId'][:35]} | {r.get('fileCount',0)} files | {r.get('totalSize',0)}B")
    for f in r.get("files", []):
        print(f"    {f['fileName']:30} {f['sizeBytes']}B")
