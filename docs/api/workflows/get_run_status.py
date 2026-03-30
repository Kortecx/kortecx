"""Get workflow run status."""
import requests, json, sys

wf_id = sys.argv[1] if len(sys.argv) > 1 else None
url = f"http://localhost:3000/api/workflows/runs?workflowId={wf_id}&limit=5" if wf_id else "http://localhost:3000/api/workflows/runs?limit=10"
resp = requests.get(url)
runs = resp.json().get("runs", [])
for r in runs:
    print(f"  {r['id'][:35]:35} status={r.get('status'):10} tokens={r.get('totalTokensUsed',0):6} chain={r.get('expertChain',[])}")
