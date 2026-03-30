"""Trigger an agent execution run."""
import requests, json, sys

agent_id = sys.argv[1] if len(sys.argv) > 1 else "local-fastapi-cli-generator"
prompt = sys.argv[2] if len(sys.argv) > 2 else "Write a hello world FastAPI app."

body = {
    "expertId": agent_id,
    "expertName": agent_id,
    "model": "llama3.2:3b",
    "engine": "ollama",
    "temperature": 0.7,
    "maxTokens": 4096,
    "systemPrompt": "You are an expert coder.",
    "userPrompt": prompt,
    "tags": ["api-test"],
}

resp = requests.post("http://localhost:3000/api/experts/run", json=body)
data = resp.json()
print(f"✓ Run started: {data.get('runId')}" if resp.ok else f"✗ Failed: {data.get('error')}")
