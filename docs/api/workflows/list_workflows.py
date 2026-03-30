"""List all workflows."""
import requests

resp = requests.get("http://localhost:3000/api/workflows")
data = resp.json()
wfs = data.get("workflows", [])
print(f"Total: {len(wfs)}\n")
for w in wfs:
    print(f"  {w['id']:20} {w['name']:30} status={w.get('status','?'):10} runs={w.get('totalRuns',0)}")
