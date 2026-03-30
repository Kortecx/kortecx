"""Trigger multiple workflows in parallel."""
import requests, json, time, concurrent.futures

ENGINE = "http://localhost:8000"

workflows = [
    {"name": "Task 1 - Code Gen", "task": "Generate a Python fibonacci function."},
    {"name": "Task 2 - Docs", "task": "Write API documentation for a user service."},
    {"name": "Task 3 - Review", "task": "Review this code: def add(a,b): return a+b"},
]

def run_workflow(wf):
    body = {
        "name": wf["name"], "workflowId": f"wf-parallel-{int(time.time()*1000)%100000}",
        "goalFileUrl": "",
        "steps": [{"stepId": "s1", "name": "Execute", "taskDescription": wf["task"],
                    "modelSource": "local", "localModel": {"engine": "ollama", "model": "llama3.2:3b"},
                    "temperature": 0.7, "maxTokens": 1024, "connectionType": "sequential"}],
    }
    resp = requests.post(f"{ENGINE}/api/orchestrator/execute", json=body)
    return f"✓ {wf['name']}: {resp.json().get('runId', '?')}"

print("Launching workflows in parallel...")
with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
    results = pool.map(run_workflow, workflows)
    for r in results:
        print(f"  {r}")

print("\nPolling completion...")
for i in range(20):
    time.sleep(5)
    st = requests.get(f"{ENGINE}/api/orchestrator/status").json()
    print(f"  [{(i+1)*5}s] active={st['active_runs']}")
    if st["active_runs"] == 0:
        print("✓ All completed"); break
