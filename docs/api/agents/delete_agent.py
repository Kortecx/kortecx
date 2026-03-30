"""Delete an agent by ID."""
import requests, sys

agent_id = sys.argv[1] if len(sys.argv) > 1 else None
if not agent_id:
    print("Usage: python delete_agent.py <agent_id>"); exit(1)

resp = requests.delete(f"http://localhost:3000/api/experts?id={agent_id}")
data = resp.json()
print(f"✓ Deleted: {data.get('deleted', False)}" if resp.ok else f"✗ Failed: {data.get('error')}")
