"""Create an agent group."""
import requests, json, sys

name = sys.argv[1] if len(sys.argv) > 1 else "Code Review Team"
agent_ids = sys.argv[2:] if len(sys.argv) > 2 else ["local-fastapi-cli-generator", "local-project-orchestrator"]

body = {"name": name, "description": f"Agent group: {name}", "agentIds": agent_ids}
resp = requests.post("http://localhost:3000/api/experts/groups", json=body)
data = resp.json()
g = data.get("group", {})
print(f"✓ Group created: {g.get('id')}, agents: {g.get('agentIds')}" if resp.ok else f"✗ Failed: {data.get('error')}")
