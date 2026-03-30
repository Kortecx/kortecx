"""Trigger workflow execution via engine."""
import requests, json, sys, time

ENGINE = "http://localhost:8000"
FRONTEND = "http://localhost:3000"

wf_name = sys.argv[1] if len(sys.argv) > 1 else "Test Workflow"
goal = sys.argv[2] if len(sys.argv) > 2 else "Generate a simple Python script."

# Upload goal file
with open("/tmp/_goal.md", "w") as f:
    f.write(goal)
upload = requests.post(f"{ENGINE}/api/orchestrator/upload", files={"files": open("/tmp/_goal.md", "rb")})
goal_url = upload.json().get("files", [{}])[0].get("url", "")
print(f"Goal uploaded: {goal_url}")

# Execute
body = {
    "name": wf_name,
    "workflowId": f"wf-api-{int(time.time())}",
    "goalFileUrl": goal_url,
    "steps": [{
        "stepId": "s1", "name": "Execute",
        "taskDescription": goal,
        "modelSource": "local",
        "localModel": {"engine": "ollama", "model": "llama3.2:3b"},
        "temperature": 0.7, "maxTokens": 2048, "connectionType": "sequential",
    }],
}

resp = requests.post(f"{ENGINE}/api/orchestrator/execute", json=body)
data = resp.json()
run_id = data.get("runId", "?")
print(f"✓ Run started: {run_id}")

# Poll
for i in range(30):
    time.sleep(5)
    st = requests.get(f"{ENGINE}/api/orchestrator/status").json()
    active = st.get("active_runs", 0)
    print(f"  [{(i+1)*5}s] active={active}")
    if active == 0:
        print("✓ Completed")
        break
