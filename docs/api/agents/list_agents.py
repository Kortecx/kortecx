"""List all agents from the Kortecx API."""
import requests, json

resp = requests.get("http://localhost:3000/api/experts")
data = resp.json()
agents = data.get("experts", [])
print(f"Total agents: {data.get('total', len(agents))}\n")
for a in agents:
    caps = a.get("capabilities") or []
    print(f"  {a['id']:35} {a['name']:25} role={a['role']:12} caps={caps}")
