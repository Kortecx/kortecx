"""Upload a goal file for workflow execution."""
import requests, sys

file_path = sys.argv[1] if len(sys.argv) > 1 else None
if not file_path:
    print("Usage: python upload_goal.py <file_path>"); exit(1)

resp = requests.post("http://localhost:8000/api/orchestrator/upload", files={"files": open(file_path, "rb")})
data = resp.json()
files = data.get("files", [])
for f in files:
    print(f"✓ Uploaded: {f['filename']} → {f['url']} ({f['size']}B)")
