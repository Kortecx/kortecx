"""Connect to external data source via plugin step."""
import requests, json, sys

wf_id = sys.argv[1] if len(sys.argv) > 1 else None
source_type = sys.argv[2] if len(sys.argv) > 2 else "database"

if not wf_id:
    print("Usage: python connect_external.py <workflow_id> [source_type]"); exit(1)

sources = {
    "database": {"name": "DB Query", "config": {"type": "postgresql", "query": "SELECT * FROM users LIMIT 10"}},
    "api": {"name": "REST API", "config": {"url": "https://api.example.com/data", "method": "GET", "headers": {}}},
    "s3": {"name": "S3 Bucket", "config": {"bucket": "my-data", "prefix": "inputs/", "region": "us-east-1"}},
    "gcs": {"name": "GCS Bucket", "config": {"bucket": "my-data", "prefix": "inputs/"}},
}

src = sources.get(source_type)
if not src:
    print(f"✗ Unknown source: {source_type}. Options: {list(sources.keys())}"); exit(1)

step = {
    "order": 0, "name": f"Fetch from {src['name']}",
    "stepType": "action",
    "actionConfig": {"transformerType": "executable", "executionRuntime": "python", "outputFormat": "markdown",
                     "externalSource": src["config"]},
    "taskDescription": f"Fetch data from {src['name']}: {json.dumps(src['config'])}",
    "modelSource": "local", "localModelConfig": {"engine": "ollama", "modelName": "llama3.2:3b"},
    "connectionType": "sequential", "temperature": 0.7, "maxTokens": 2048,
}

wf = requests.get(f"http://localhost:3000/api/workflows?id={wf_id}").json().get("workflow", {})
steps = wf.get("steps", [])
step["order"] = 1
for s in steps:
    s["order"] = s.get("order", 1) + 1  # shift existing
steps.insert(0, step)

resp = requests.patch("http://localhost:3000/api/workflows", json={"id": wf_id, "steps": steps})
print(f"✓ External source '{src['name']}' connected as first step" if resp.ok else f"✗ Failed")
