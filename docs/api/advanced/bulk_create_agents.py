"""Create multiple agents at once."""
import requests, json

agents = [
    {"name": "Code Writer", "role": "coder", "description": "Generates production code", "tags": ["python"]},
    {"name": "Code Reviewer", "role": "reviewer", "description": "Reviews code for quality", "tags": ["review"]},
    {"name": "Doc Writer", "role": "writer", "description": "Generates documentation", "tags": ["docs"]},
    {"name": "Test Engineer", "role": "coder", "description": "Writes unit tests", "tags": ["testing"]},
]

for a in agents:
    body = {
        **a,
        "systemPrompt": f"You are a {a['role']} specializing in {a['description'].lower()}.",
        "modelSource": "local",
        "localModelConfig": {"engine": "ollama", "modelName": "llama3.2:3b"},
        "temperature": 0.7, "maxTokens": 4096,
        "capabilities": [a["role"]], "category": "custom", "complexityLevel": 3,
    }
    resp = requests.post("http://localhost:3000/api/experts", json=body)
    e = resp.json().get("expert", {})
    status = "✓" if resp.ok else "✗"
    print(f"  {status} {a['name']:20} → {e.get('id', 'FAILED')}")
