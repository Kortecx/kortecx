"""Get a single agent by ID."""
import requests, json, sys

agent_id = sys.argv[1] if len(sys.argv) > 1 else "local-fastapi-cli-generator"
resp = requests.get(f"http://localhost:3000/api/experts?id={agent_id}")
data = resp.json()
e = data.get("expert", {})
if e:
    print(json.dumps(e, indent=2, default=str))
else:
    print(f"✗ Agent '{agent_id}' not found")
