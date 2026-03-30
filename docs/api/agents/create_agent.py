"""Create a new agent via the Kortecx API."""
import requests, json, sys

FRONTEND = "http://localhost:3000"

agent = {
    "name": sys.argv[1] if len(sys.argv) > 1 else "Sample Agent",
    "role": sys.argv[2] if len(sys.argv) > 2 else "coder",
    "description": "A versatile AI agent for code generation and review.",
    "systemPrompt": "You are an expert software engineer. Generate clean, production-ready code with proper error handling.",
    "modelSource": "local",
    "localModelConfig": {"engine": "ollama", "modelName": "llama3.2:3b"},
    "temperature": 0.7,
    "maxTokens": 4096,
    "tags": ["python", "code-gen"],
    "capabilities": ["code-generation", "review"],
    "specializations": ["Python", "FastAPI"],
    "category": "custom",
    "complexityLevel": 3,
}

resp = requests.post(f"{FRONTEND}/api/experts", json=agent)
data = resp.json()
if resp.ok:
    e = data.get("expert", {})
    print(f"✓ Agent created: {e.get('id')}")
    print(f"  Name: {e.get('name')}, Role: {e.get('role')}")
else:
    print(f"✗ Failed: {data.get('error', resp.text)}")
