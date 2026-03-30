# Kortecx API Reference Scripts

Standalone Python and TypeScript scripts for all major agent and workflow operations.

## Prerequisites
- Engine running: `http://localhost:8000`
- Frontend running: `http://localhost:3000`
- Python: `pip install requests`
- TypeScript: `npx tsx` (already in project)

## Agents

| Script | Description | Usage |
|--------|-------------|-------|
| `agents/create_agent.py` | Create agent | `python docs/api/agents/create_agent.py "My Agent" coder` |
| `agents/list_agents.py` | List all agents | `python docs/api/agents/list_agents.py` |
| `agents/get_agent.py` | Get agent by ID | `python docs/api/agents/get_agent.py local-my-agent` |
| `agents/delete_agent.py` | Delete agent | `python docs/api/agents/delete_agent.py local-my-agent` |
| `agents/run_agent.py` | Run agent | `python docs/api/agents/run_agent.py local-my-agent "Write code"` |
| `agents/create_group.py` | Create group | `python docs/api/agents/create_group.py "Team" id1 id2` |
| `agents/agent_outputs.py` | List outputs | `python docs/api/agents/agent_outputs.py local-my-agent` |

## Workflows

| Script | Description | Usage |
|--------|-------------|-------|
| `workflows/create_workflow.py` | Create workflow | `python docs/api/workflows/create_workflow.py "My Pipeline"` |
| `workflows/list_workflows.py` | List workflows | `python docs/api/workflows/list_workflows.py` |
| `workflows/run_workflow.py` | Execute via engine | `python docs/api/workflows/run_workflow.py "Pipeline" "Generate code"` |
| `workflows/get_run_status.py` | Run status | `python docs/api/workflows/get_run_status.py wf-id` |
| `workflows/get_run_outputs.py` | Run outputs | `python docs/api/workflows/get_run_outputs.py workflow-name` |
| `workflows/cancel_run.py` | Cancel run | `python docs/api/workflows/cancel_run.py run-id wf-id` |
| `workflows/delete_run.py` | Delete run | `python docs/api/workflows/delete_run.py run-id` |
| `workflows/save_config.py` | Save config | `python docs/api/workflows/save_config.py workflow-name` |

## Advanced

| Script | Description | Usage |
|--------|-------------|-------|
| `advanced/add_steps.py` | Add steps to workflow | `python docs/api/advanced/add_steps.py wf-id` |
| `advanced/update_config.py` | Update config | `python docs/api/advanced/update_config.py wf-id` |
| `advanced/parallel_run.py` | Run multiple in parallel | `python docs/api/advanced/parallel_run.py` |
| `advanced/configure_integration.py` | Add integration | `python docs/api/advanced/configure_integration.py wf-id slack` |
| `advanced/connect_plugin.py` | Attach plugin | `python docs/api/advanced/connect_plugin.py wf-id web-scraper` |
| `advanced/connect_external.py` | External source | `python docs/api/advanced/connect_external.py wf-id database` |
| `advanced/upload_goal.py` | Upload goal file | `python docs/api/advanced/upload_goal.py goal.md` |
| `advanced/list_versions.py` | List versions | `python docs/api/advanced/list_versions.py workflow-name` |
| `advanced/revert_version.py` | Revert version | `python docs/api/advanced/revert_version.py name timestamp` |
| `advanced/bulk_create_agents.py` | Bulk create agents | `python docs/api/advanced/bulk_create_agents.py` |

## TypeScript

| Script | Description | Usage |
|--------|-------------|-------|
| `typescript/create_agent.ts` | Create agent | `npx tsx docs/api/typescript/create_agent.ts "Agent"` |
| `typescript/create_workflow.ts` | Create workflow | `npx tsx docs/api/typescript/create_workflow.ts "Pipeline"` |
| `typescript/run_workflow.ts` | Execute workflow | `npx tsx docs/api/typescript/run_workflow.ts "Generate code"` |
| `typescript/get_outputs.ts` | Get outputs | `npx tsx docs/api/typescript/get_outputs.ts workflow-name` |

## API Endpoints Reference

### Frontend (localhost:3000)
| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/experts` | List agents |
| GET | `/api/experts?id={id}` | Get agent |
| POST | `/api/experts` | Create agent |
| PATCH | `/api/experts` | Update agent |
| DELETE | `/api/experts?id={id}` | Delete agent |
| POST | `/api/experts/run` | Run agent |
| GET | `/api/experts/outputs?expertId={id}` | Agent outputs |
| GET/POST/DELETE | `/api/experts/groups` | Agent groups |
| GET | `/api/workflows` | List workflows |
| POST | `/api/workflows` | Create workflow |
| PATCH | `/api/workflows` | Update workflow |
| DELETE | `/api/workflows?id={id}` | Delete workflow |
| POST | `/api/workflows/run` | Create run record |
| GET | `/api/workflows/runs?workflowId={id}` | Run history |
| DELETE | `/api/workflows/runs?id={id}` | Delete run |
| POST | `/api/workflows/stop` | Cancel run |
| GET | `/api/workflows/outputs?workflowName={n}` | Workflow outputs |
| POST | `/api/workflows/save-local` | Save config locally |

### Engine (localhost:8000)
| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/orchestrator/execute` | Execute workflow |
| GET | `/api/orchestrator/status` | Engine status |
| GET | `/api/orchestrator/runs/{runId}` | Run details |
| POST | `/api/orchestrator/runs/{runId}/cancel` | Cancel run |
| POST | `/api/orchestrator/upload` | Upload files |
| GET | `/api/orchestrator/outputs/{name}` | Workflow outputs |
| POST | `/api/orchestrator/save-config` | Save versioned config |
| POST | `/api/orchestrator/workflow-save-local` | Save local config |
| GET | `/api/orchestrator/workflow-versions/{name}` | List versions |
| GET | `/api/orchestrator/health/ollama` | Ollama health |
| GET | `/api/orchestrator/models/ollama` | Available models |
| GET | `/api/agents/engine/create` | Create agent (engine) |
| GET | `/api/agents/engine/list` | List agents (engine) |
| GET | `/api/agents/engine/{id}/outputs` | Agent outputs (engine) |
