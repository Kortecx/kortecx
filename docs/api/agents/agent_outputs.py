"""List agent run outputs."""
import requests, json, sys

agent_id = sys.argv[1] if len(sys.argv) > 1 else "local-fastapi-cli-generator"
resp = requests.get(f"http://localhost:3000/api/experts/outputs?expertId={agent_id}")
data = resp.json()
runs = data.get("runs", [])
print(f"Output runs: {len(runs)}\n")
for r in runs[:10]:
    print(f"  {r.get('runTs','?'):20} files={r.get('fileCount',0):3} size={r.get('totalSize',0):6}B")
    for f in r.get("files", [])[:3]:
        print(f"    {f['fileName']:30} {f['sizeBytes']}B")
