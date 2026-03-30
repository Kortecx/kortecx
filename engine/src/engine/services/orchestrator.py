"""Agent orchestrator — spawns agents, manages shared memory, coordinates execution."""

from __future__ import annotations

import asyncio
import json
import logging
import uuid
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import httpx

from engine.config import settings
from engine.core.websocket import ws_manager
from engine.services.action_runner import action_runner
from engine.services.execution_audit import execution_audit
from engine.services.local_inference import GenerateResult, inference_router, model_pool
from engine.services.step_artifacts import step_artifacts
from engine.services.workflow_artifacts import workflow_artifacts
from engine.services.workflow_logger import workflow_logger

logger = logging.getLogger("engine.orchestrator")


@dataclass
class AgentMemory:
    plan: str = ""
    context: str = ""
    findings: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {"plan": self.plan, "context": self.context, "findings": self.findings}


@dataclass
class SharedMemory:
    run_id: str
    entries: dict[str, str] = field(default_factory=dict)  # agentId -> JSON
    globals: dict[str, str] = field(default_factory=dict)  # shared KV

    def to_dict(self) -> dict[str, Any]:
        return {"runId": self.run_id, "entries": self.entries, "globals": self.globals}


@dataclass
class StepIntegration:
    id: str
    type: str  # "integration" | "plugin"
    reference_id: str
    name: str
    icon: str = ""
    color: str = ""
    config: dict[str, str] = field(default_factory=dict)


@dataclass
class StepConfig:
    step_id: str
    expert_id: str | None
    task_description: str
    model_source: str  # "local" | "provider"
    local_model: dict[str, Any] | None
    temperature: float
    max_tokens: int
    connection_type: str  # "sequential" | "parallel" | "conditional"
    step_name: str = ""
    system_instructions: str = ""
    voice_command: str = ""
    file_locations: list[str] = field(default_factory=list)
    step_file_names: list[str] = field(default_factory=list)
    step_image_names: list[str] = field(default_factory=list)
    integrations: list[StepIntegration] = field(default_factory=list)
    condition: dict[str, str] | None = None  # e.g. {"type": "contains", "value": "APPROVED"}
    step_type: str = "agent"  # "agent" | "action"
    action_config: dict[str, Any] | None = None
    max_retries: int = 0
    retry_delay_sec: int = 2


@dataclass
class WorkflowRequest:
    workflow_id: str
    name: str
    goal_file_url: str
    input_file_urls: list[str]
    steps: list[StepConfig]
    master_agent_id: str | None = None
    connected_agent_ids: list[str] = field(default_factory=list)
    fail_fast: bool = False


@dataclass
class AgentState:
    agent_id: str
    step_id: str
    status: str = "pending"  # pending | running | completed | failed
    memory: AgentMemory = field(default_factory=AgentMemory)
    output: str = ""
    error: str = ""
    tokens_used: int = 0
    duration_ms: float = 0


class AgentOrchestrator:
    """Core orchestration runtime for workflow agent execution."""

    FALLBACK_MODELS = {
        "ollama": "llama3.2:3b",
        "llamacpp": "default",
    }

    def __init__(self) -> None:
        self._runs: dict[str, dict[str, Any]] = {}  # runId -> run state
        self._shared_memory: dict[str, SharedMemory] = {}
        self._semaphore = asyncio.Semaphore(settings.max_concurrent_agents)
        self._cancellation_events: dict[str, asyncio.Event] = {}
        self._metrics_stop_events: dict[str, asyncio.Event] = {}

    def _plan_dag_to_steps(self, dag: dict[str, Any]) -> list[StepConfig]:
        """Convert a plan DAG into a list of StepConfigs with correct connection types.

        Performs topological ordering and groups parallel-capable nodes.
        """
        nodes = dag.get("nodes", [])
        edges = dag.get("edges", [])

        if not nodes:
            return []

        # Build dependency map: node_id -> list of dependency node_ids
        deps: dict[str, list[str]] = {n["id"]: [] for n in nodes}
        for edge in edges:
            src, tgt = edge.get("source"), edge.get("target")
            if tgt in deps:
                deps[tgt].append(src)

        # Topological sort into layers
        node_map = {n["id"]: n for n in nodes}
        in_degree = {nid: len(d) for nid, d in deps.items()}
        queue = [nid for nid in in_degree if in_degree[nid] == 0]
        layers: list[list[str]] = []

        while queue:
            layers.append(list(queue))
            next_queue: list[str] = []
            for nid in queue:
                for dep_nid, dep_list in deps.items():
                    if nid in dep_list:
                        in_degree[dep_nid] -= 1
                        if in_degree[dep_nid] == 0:
                            next_queue.append(dep_nid)
            queue = next_queue

        # Convert layers to StepConfigs
        steps: list[StepConfig] = []
        for layer in layers:
            conn_type = "parallel" if len(layer) > 1 else "sequential"
            for nid in layer:
                node = node_map.get(nid, {})
                steps.append(
                    StepConfig(
                        step_id=nid,
                        expert_id=node.get("agentId") or node.get("expertId"),
                        task_description=node.get("description", ""),
                        model_source="local",
                        local_model=None,
                        temperature=0.7,
                        max_tokens=4096,
                        connection_type=conn_type,
                        step_name=node.get("label", nid),
                        system_instructions=node.get("systemInstructions", ""),
                    )
                )

        return steps

    async def execute_workflow(self, request: WorkflowRequest, run_id: str | None = None, plan: dict[str, Any] | None = None) -> dict[str, Any]:
        """Main entry point — creates a run and orchestrates agents.

        If a plan DAG is provided, it overrides request.steps with the DAG-derived step order.
        """
        if not run_id:
            slug = request.name.lower().replace(" ", "-").replace("_", "-")[:30] if request.name else "wf"
            slug = "".join(c for c in slug if c.isalnum() or c == "-")
            ts = datetime.now(UTC).strftime("%Y%m%d-%H%M%S")
            run_id = f"wfr-{slug}-{ts}-{uuid.uuid4().hex[:4]}"
        channel = f"workflow.{run_id}"

        # If a plan DAG is provided, convert it to steps and override request
        if plan and plan.get("nodes"):
            plan_steps = self._plan_dag_to_steps(plan)
            if plan_steps:
                logger.info("Using plan DAG with %d steps for run %s", len(plan_steps), run_id)
                request = WorkflowRequest(
                    workflow_id=request.workflow_id,
                    name=request.name,
                    goal_file_url=request.goal_file_url,
                    input_file_urls=request.input_file_urls,
                    steps=plan_steps,
                )

        shared = SharedMemory(run_id=run_id)
        self._shared_memory[run_id] = shared
        self._cancellation_events[run_id] = asyncio.Event()
        self._metrics_stop_events[run_id] = asyncio.Event()

        # Read goal file content
        goal_content = await self._read_file(request.goal_file_url)

        # Read input files
        input_contents: dict[str, str] = {}
        for url in request.input_file_urls:
            content = await self._read_file(url)
            filename = Path(url).name
            input_contents[filename] = content

        # Build run state
        self._runs[run_id] = {
            "id": run_id,
            "workflowId": request.workflow_id,
            "name": request.name,
            "status": "running",
            "startedAt": datetime.now(UTC).isoformat(),
            "agents": {},
            "stepResults": {},
            "request": request,
        }

        # Log run start
        workflow_logger.log_run_event(
            request.workflow_id,
            run_id,
            "run.started",
            {
                "name": request.name,
                "totalSteps": len(request.steps),
            },
        )

        # Broadcast run started
        await ws_manager.broadcast(
            channel,
            "run.started",
            {
                "runId": run_id,
                "name": request.name,
                "totalSteps": len(request.steps),
            },
        )

        # Create audit trail run record
        await execution_audit.create_run(run_id, request.name, request.steps)

        # Orchestrate
        try:
            result = await self._orchestrate(
                run_id,
                request.steps,
                shared,
                goal_content,
                input_contents,
                channel,
            )
            self._runs[run_id]["status"] = "completed"
            self._runs[run_id]["completedAt"] = datetime.now(UTC).isoformat()

            # Log completion
            workflow_logger.log_run_event(
                request.workflow_id,
                run_id,
                "run.completed",
                {
                    "outputLength": len(result),
                },
            )
            workflow_logger.save_run_memory(request.workflow_id, run_id, shared.to_dict())
            workflow_logger.save_run_output(request.workflow_id, run_id, result)

            # Save structured outputs to outputs/workflows/
            total_tokens_out, total_dur_out, succ_out, fail_out = self._compute_run_stats(run_id)
            agents_data = self._runs[run_id].get("agents", {})
            expert_chain_out = [a.step_id for a in agents_data.values()] if agents_data else []
            try:
                workflow_artifacts.save_run_summary(
                    workflow_name=request.name,
                    workflow_id=request.workflow_id,
                    run_id=run_id,
                    output=result,
                    goal=goal_content,
                    status="completed",
                    total_tokens=total_tokens_out,
                    duration_sec=int(total_dur_out / 1000) if total_dur_out else 0,
                    steps_completed=succ_out,
                    total_steps=len(request.steps),
                    expert_chain=expert_chain_out,
                )
                # Save per-step outputs
                for idx, step in enumerate(request.steps):
                    agent_id = step.agent_id or step.expert_id or f"step-{idx}"
                    agent_state = agents_data.get(agent_id)
                    if agent_state:
                        workflow_artifacts.save_step_output(
                            workflow_name=request.name,
                            run_id=run_id,
                            step_number=idx + 1,
                            step_name=step.name or agent_id,
                            response=agent_state.output or "",
                            prompt=step.task or "",
                            system_prompt=step.system_prompt or "",
                            model=step.model or "",
                            engine=step.engine or "",
                            tokens_used=agent_state.tokens_used,
                            duration_ms=agent_state.duration_ms,
                            status=agent_state.status,
                            error_message=agent_state.error or "",
                        )
            except Exception:
                logger.warning("Failed to save workflow artifacts for %s", run_id, exc_info=True)

            # Master agent post-processing: collect all outputs and produce refined summary
            if request.master_agent_id:
                try:
                    from engine.services.expert_manager import expert_manager

                    master_expert = expert_manager.get(request.master_agent_id)
                    if master_expert:
                        master_system = expert_manager.get_prompt(request.master_agent_id, "system")
                        agents_data = self._runs[run_id].get("agents", {})
                        step_outputs = "\n\n---\n\n".join(
                            f"## Step: {a.name}\n\n{a.output}" for a in agents_data.values() if a.output
                        )
                        master_prompt = (
                            f"You are the master agent for workflow '{request.name}'.\n\n"
                            f"Goal: {goal_content}\n\n"
                            f"All step outputs are below. Synthesize, refine, and enhance them into a final output.\n\n"
                            f"{step_outputs}"
                        )
                        master_model = (master_expert.get("localModelConfig") or {}).get("modelName", "llama3.2:3b")
                        master_engine = (master_expert.get("localModelConfig") or {}).get("engine", "ollama")

                        master_result = await inference_router.chat(
                            engine=master_engine,
                            model=master_model,
                            messages=[
                                {"role": "system", "content": master_system or "You are a master agent that synthesizes and refines outputs."},
                                {"role": "user", "content": master_prompt},
                            ],
                            temperature=0.7,
                            max_tokens=8192,
                        )
                        result = f"# Master Agent Output\n\n{master_result.text}\n\n---\n\n# Step Outputs\n\n{result}"
                        workflow_artifacts.save_run_summary(
                            workflow_name=request.name,
                            workflow_id=request.workflow_id,
                            run_id=run_id,
                            output=master_result.text,
                            goal=goal_content,
                            status="completed",
                            metadata={"masterAgentId": request.master_agent_id},
                        )
                        logger.info("Master agent processed outputs for run %s", run_id)
                except Exception:
                    logger.warning("Master agent post-processing failed for %s", run_id, exc_info=True)

            await ws_manager.broadcast(
                channel,
                "workflow.complete",
                {
                    "runId": run_id,
                    "output": result,
                    "sharedMemory": shared.to_dict(),
                },
            )

            # Persist completion log to frontend
            try:
                async with httpx.AsyncClient(timeout=5) as _client:
                    await _client.post(
                        "http://localhost:3000/api/logs",
                        json={
                            "level": "info",
                            "message": f"Workflow '{request.name}' completed successfully",
                            "source": "workflow",
                            "runId": run_id,
                            "metadata": {
                                "workflowId": request.workflow_id,
                                "workflowName": request.name,
                                "totalSteps": len(request.steps),
                                "outputLength": len(result),
                            },
                        },
                    )
            except Exception:
                pass  # Non-critical

            # Persist audit trail
            total_tokens, total_duration, succeeded, failed = self._compute_run_stats(run_id)
            await execution_audit.complete_run(
                run_id,
                total_tokens,
                total_duration,
                result,
                succeeded,
                failed,
            )
            await execution_audit.save_shared_memory(run_id, "complete", shared.to_dict())
            await self._update_expert_stats(run_id)

            # Sync run + step execution data to frontend DB
            await self._sync_to_frontend(
                run_id=run_id,
                workflow_id=request.workflow_id,
                workflow_name=request.name,
                status="completed",
                steps=request.steps,
            )

            return {"runId": run_id, "status": "completed", "output": result}

        except asyncio.CancelledError:
            self._runs[run_id]["status"] = "cancelled"
            self._runs[run_id]["completedAt"] = datetime.now(UTC).isoformat()

            workflow_logger.log_run_event(
                request.workflow_id,
                run_id,
                "run.cancelled",
                {},
            )
            await ws_manager.broadcast(
                channel,
                "workflow.cancelled",
                {"runId": run_id, "message": "Workflow cancelled by user"},
            )
            await execution_audit.fail_run(run_id, "Cancelled by user")
            await self._sync_to_frontend(
                run_id=run_id,
                workflow_id=request.workflow_id,
                workflow_name=request.name,
                status="cancelled",
                steps=request.steps,
                error_message="Cancelled by user",
            )
            return {"runId": run_id, "status": "cancelled"}

        except Exception as exc:
            logger.exception("Workflow %s failed", run_id)
            self._runs[run_id]["status"] = "failed"
            self._runs[run_id]["error"] = str(exc)

            workflow_logger.log_run_event(
                request.workflow_id,
                run_id,
                "run.failed",
                {
                    "error": str(exc),
                },
            )

            await ws_manager.broadcast(
                channel,
                "workflow.failed",
                {
                    "runId": run_id,
                    "error": str(exc),
                },
            )

            # Persist audit trail failure
            await execution_audit.fail_run(run_id, str(exc))

            # Persist workflow failure artifacts
            try:
                step_artifacts.save_failure_log(
                    workflow_name=request.name,
                    step_name="_workflow",
                    run_id=run_id,
                    error=str(exc),
                    phase="workflow",
                    metadata={"totalSteps": len(request.steps)},
                )
            except Exception:
                pass  # Non-critical

            # Persist failure log to frontend
            try:
                async with httpx.AsyncClient(timeout=5) as _client:
                    await _client.post(
                        "http://localhost:3000/api/logs",
                        json={
                            "level": "error",
                            "message": f"Workflow '{request.name}' failed: {str(exc)[:200]}",
                            "source": "workflow",
                            "runId": run_id,
                            "metadata": {
                                "workflowId": request.workflow_id,
                                "workflowName": request.name,
                                "error": str(exc),
                            },
                        },
                    )
            except Exception:
                pass  # Non-critical

            # Sync run + step execution data to frontend DB
            await self._sync_to_frontend(
                run_id=run_id,
                workflow_id=request.workflow_id,
                workflow_name=request.name,
                status="failed",
                steps=request.steps,
                error_message=str(exc),
            )

            return {"runId": run_id, "status": "failed", "error": str(exc)}

        finally:
            # Cleanup cancellation and metrics stop events
            self._cancellation_events.pop(run_id, None)
            stop_evt = self._metrics_stop_events.pop(run_id, None)
            if stop_evt:
                stop_evt.set()

    async def _orchestrate(
        self,
        run_id: str,
        steps: list[StepConfig],
        shared: SharedMemory,
        goal_content: str,
        input_contents: dict[str, str],
        channel: str,
    ) -> str:
        """Execute steps respecting connection types (sequential / parallel / conditional)."""
        # Group consecutive parallel steps
        groups: list[list[StepConfig]] = []
        current_group: list[StepConfig] = []

        for step in steps:
            if step.connection_type == "parallel":
                current_group.append(step)
            else:
                if current_group:
                    groups.append(current_group)
                    current_group = []
                groups.append([step])
        if current_group:
            groups.append(current_group)

        previous_output = ""

        for group in groups:
            # Check cancellation before each step group
            if self._cancellation_events.get(run_id, asyncio.Event()).is_set():
                raise asyncio.CancelledError(f"Workflow {run_id} cancelled by user")

            if len(group) == 1 and group[0].connection_type != "parallel":
                # Sequential / conditional execution
                step = group[0]

                # Handle conditional steps via expression-based evaluation
                if step.connection_type == "conditional":
                    should_run = self._evaluate_condition(step, previous_output, run_id)
                    if not should_run:
                        logger.info("Skipping conditional step %s (condition not met)", step.step_id)
                        self._runs[run_id]["stepResults"][step.step_id] = "[SKIPPED]"
                        continue

                if step.step_type == "action":
                    agent_state = await self._run_action(run_id, step, shared, previous_output, channel)
                else:
                    agent_state = await self._run_agent(
                        run_id,
                        step,
                        shared,
                        goal_content,
                        input_contents,
                        previous_output,
                        channel,
                    )
                previous_output = agent_state.output
                self._runs[run_id]["stepResults"][step.step_id] = agent_state.output
            else:
                # Parallel execution
                async def _run_step(s: StepConfig) -> AgentState:
                    if s.step_type == "action":
                        return await self._run_action(run_id, s, shared, previous_output, channel)
                    return await self._run_agent(run_id, s, shared, goal_content, input_contents, previous_output, channel)

                tasks = [_run_step(step) for step in group]
                results = await asyncio.gather(*tasks, return_exceptions=True)
                outputs = []
                for i, r in enumerate(results):
                    if isinstance(r, Exception):
                        logger.error("Parallel agent failed: %s", r)
                        outputs.append(f"[Step {group[i].step_id} failed: {r}]")
                    else:
                        outputs.append(r.output)
                        self._runs[run_id]["stepResults"][group[i].step_id] = r.output
                previous_output = "\n\n---\n\n".join(outputs)

        return previous_output

    async def _run_action(
        self,
        run_id: str,
        step: StepConfig,
        shared: SharedMemory,
        previous_output: str,
        channel: str,
    ) -> AgentState:
        """Execute an action step — generate files without LLM inference."""
        agent_id = f"action-{uuid.uuid4().hex[:8]}"
        agent = AgentState(agent_id=agent_id, step_id=step.step_id, status="running")
        start = datetime.now(UTC)

        step_name = step.step_name or step.step_id
        workflow_name = self._runs[run_id].get("name", "unknown")
        workflow_id = self._runs[run_id].get("workflowId", "")

        # Broadcast action started
        await ws_manager.broadcast(
            channel,
            "action.started",
            {
                "runId": run_id,
                "agentId": agent_id,
                "stepId": step.step_id,
                "stepName": step_name,
                "stepType": "action",
                "actionConfig": step.action_config,
            },
        )

        try:
            config = step.action_config or {}
            result = await action_runner.run(
                previous_output=previous_output,
                action_config=config,
                workflow_name=workflow_name,
                step_name=step_name,
                run_id=run_id,
            )

            if not result.success:
                raise RuntimeError(result.error)

            # Register the generated file as an asset
            try:
                async with httpx.AsyncClient(timeout=15) as client:
                    await client.post(
                        "http://localhost:3000/api/assets/register",
                        json={
                            "assets": [
                                {
                                    "name": result.file_name,
                                    "fileName": result.file_name,
                                    "filePath": result.file_path,
                                    "sizeBytes": result.size_bytes,
                                    "mimeType": result.mime_type,
                                    "fileType": "document",
                                    "sourceType": "workflow",
                                    "folder": f"/workflows/{workflow_name}/{step_name}",
                                    "tags": ["workflow", "action", workflow_name, step_name],
                                    "metadata": {
                                        "runId": run_id,
                                        "workflowId": workflow_id,
                                        "workflowName": workflow_name,
                                        "stepName": step_name,
                                        "stepType": "action",
                                        "outputFormat": result.output_format,
                                    },
                                }
                            ]
                        },
                    )
            except Exception as reg_exc:
                logger.warning("Failed to register action asset: %s", reg_exc)

            elapsed = (datetime.now(UTC) - start).total_seconds() * 1000
            agent.status = "completed"
            agent.output = f"[ACTION] Generated {result.file_name} ({result.size_bytes} bytes) at {result.file_path}"
            agent.duration_ms = elapsed

            # Store in shared memory for downstream steps
            shared.globals[step.step_id] = agent.output[:1000]

            await ws_manager.broadcast(
                channel,
                "action.complete",
                {
                    "runId": run_id,
                    "agentId": agent_id,
                    "stepId": step.step_id,
                    "stepName": step_name,
                    "filePath": result.file_path,
                    "fileName": result.file_name,
                    "mimeType": result.mime_type,
                    "sizeBytes": result.size_bytes,
                    "outputFormat": result.output_format,
                    "durationMs": elapsed,
                },
            )

        except Exception as exc:
            elapsed = (datetime.now(UTC) - start).total_seconds() * 1000
            agent.status = "failed"
            agent.error = str(exc)
            agent.duration_ms = elapsed
            logger.exception("Action step %s failed: %s", step.step_id, exc)

            await ws_manager.broadcast(
                channel,
                "action.failed",
                {
                    "runId": run_id,
                    "agentId": agent_id,
                    "stepId": step.step_id,
                    "stepName": step_name,
                    "error": str(exc),
                    "durationMs": elapsed,
                },
            )

        # Track agent in run state
        self._runs[run_id]["agents"][agent_id] = agent

        return agent

    async def _run_agent(
        self,
        run_id: str,
        step: StepConfig,
        shared: SharedMemory,
        goal_content: str,
        input_contents: dict[str, str],
        previous_output: str,
        channel: str,
    ) -> AgentState:
        """Spawn and execute a single agent for one workflow step."""
        agent_id = f"agent-{uuid.uuid4().hex[:8]}"
        agent = AgentState(agent_id=agent_id, step_id=step.step_id, status="running")

        # Initialize agent memory
        agent.memory.context = goal_content
        agent.memory.plan = f"Step: {step.task_description}\nGoal:\n{goal_content[:2000]}"

        self._runs[run_id]["agents"][agent_id] = agent

        # Log agent spawn
        workflow_logger.log_run_event(
            self._runs[run_id]["workflowId"],
            run_id,
            "agent.spawned",
            {"agentId": agent_id, "stepId": step.step_id, "modelSource": step.model_source},
        )

        # Broadcast agent spawned
        await ws_manager.broadcast(
            channel,
            "agent.spawned",
            {
                "runId": run_id,
                "agentId": agent_id,
                "stepId": step.step_id,
                "taskDescription": step.task_description,
                "modelSource": step.model_source,
                "stepName": getattr(step, "step_name", "") or "",
                "model": step.local_model.get("model", "") if step.local_model else "",
                "engine": step.local_model.get("engine", "") if step.local_model else "",
            },
        )

        # Audit: agent spawned
        execution_audit.log_agent_spawned(
            run_id,
            agent_id,
            step.step_id,
            expert_id=step.expert_id,
            model_source=step.model_source,
            task_description=step.task_description,
        )

        # Persist step status: pending
        await self._persist_step_status(run_id, step, agent_id, "pending")

        async with self._semaphore:
            try:
                # Resolve expert from DB if available
                expert_data = await self._resolve_expert(step)

                # If expert has local model config, use it
                if expert_data and expert_data.get("modelSource") == "local" and expert_data.get("localModelConfig"):
                    lmc = expert_data["localModelConfig"]
                    step = StepConfig(
                        step_id=step.step_id,
                        expert_id=step.expert_id,
                        task_description=step.task_description,
                        model_source="local",
                        local_model={"engine": lmc.get("engine", "ollama"), "model": lmc.get("model", lmc.get("modelName", "")), "baseUrl": lmc.get("baseUrl")},
                        temperature=expert_data.get("temperature", step.temperature) if isinstance(expert_data.get("temperature"), (int, float)) else step.temperature,
                        max_tokens=expert_data.get("maxTokens", step.max_tokens) or step.max_tokens,
                        connection_type=step.connection_type,
                    )

                # Build prompt
                system_prompt = self._build_system_prompt(step, shared, expert_data)
                user_prompt = await self._build_user_prompt(
                    step,
                    goal_content,
                    input_contents,
                    previous_output,
                    shared,
                )

                # Broadcast thinking
                import time as _time

                _thinking_start = _time.monotonic()  # noqa: F841
                await ws_manager.broadcast(
                    channel,
                    "agent.thinking",
                    {
                        "runId": run_id,
                        "agentId": agent_id,
                        "stepId": step.step_id,
                        "startedAt": datetime.now(UTC).isoformat(),
                    },
                )

                # Audit: agent thinking
                execution_audit.log_agent_thinking(run_id, agent_id, step.step_id)

                # Persist step status: running
                await self._persist_step_status(run_id, step, agent_id, "running")

                # Start live metrics broadcasting
                _metrics_task = asyncio.create_task(self._broadcast_live_metrics(run_id, agent_id, step.step_id, channel))

                # Execute inference with retry — race against cancellation event
                max_attempts = 1 + getattr(step, "max_retries", 0)
                retry_delay = getattr(step, "retry_delay_sec", 2)
                result = None
                for attempt in range(max_attempts):
                    try:
                        cancel_evt = self._cancellation_events.get(run_id)
                        if cancel_evt:
                            infer_task = asyncio.create_task(self._infer(step, system_prompt, user_prompt))
                            cancel_wait = asyncio.create_task(cancel_evt.wait())
                            done, pending = await asyncio.wait(
                                {infer_task, cancel_wait},
                                return_when=asyncio.FIRST_COMPLETED,
                            )
                            for p in pending:
                                p.cancel()
                            if cancel_wait in done:
                                raise asyncio.CancelledError(f"Workflow {run_id} cancelled by user")
                            result = infer_task.result()
                        else:
                            result = await self._infer(step, system_prompt, user_prompt)
                        break  # success
                    except asyncio.CancelledError:
                        raise  # don't retry cancellations
                    except Exception as retry_err:  # noqa: F841
                        if attempt < max_attempts - 1:
                            logger.warning("Step %s attempt %d failed, retrying in %ds: %s", step.step_id, attempt + 1, retry_delay, retry_err)
                            await asyncio.sleep(retry_delay * (2 ** attempt))  # exponential backoff
                        else:
                            raise  # final attempt, let outer handler catch

                # Stop live metrics broadcasting
                _metrics_task.cancel()

                agent.output = result.text
                agent.tokens_used = result.tokens_used
                agent.duration_ms = result.duration_ms
                agent.status = "completed"

                # Audit: inference result
                _engine = step.local_model.get("engine", "ollama") if step.local_model else "provider"
                _model = step.local_model.get("model", "") if step.local_model else "hf-fallback"
                execution_audit.log_inference(
                    run_id,
                    agent_id,
                    system_prompt,
                    user_prompt,
                    result.text,
                    result.tokens_used,
                    result.duration_ms,
                    model=_model,
                    engine=_engine,
                    temperature=step.temperature,
                    max_tokens=step.max_tokens,
                )

                # Update agent memory with findings
                agent.memory.findings.append(result.text[:500])

                # Persist artifacts to disk
                _step_name = step.step_name or step.step_id
                try:
                    step_artifacts.save_response(
                        workflow_name=self._runs[run_id].get("name", "unnamed"),
                        step_name=_step_name,
                        run_id=run_id,
                        agent_id=agent_id,
                        response=result.text,
                        prompt=user_prompt,
                        system_prompt=system_prompt,
                        model=step.local_model.get("model", "") if step.local_model else "",
                        tokens_used=result.tokens_used,
                        duration_ms=result.duration_ms,
                    )
                except Exception as artifact_exc:
                    logger.warning("Failed to save artifacts: %s", artifact_exc)

                # Extract and execute any scripts in the response
                try:
                    scripts = step_artifacts.extract_and_save_scripts(
                        self._runs[run_id].get("name", "unnamed"),
                        _step_name,
                        result.text,
                    )
                    if scripts:
                        for script in scripts:
                            if script.suffix in (".py", ".sh", ".js"):
                                try:
                                    script_result = await step_artifacts.execute_script(script, timeout=60)
                                    if script_result.get("exitCode") == 0 and script_result.get("stdout"):
                                        agent.memory.findings.append(f"[Script {script.name} output]: {script_result['stdout'][:1000]}")
                                        shared.globals[f"{step.step_id}_script_{script.name}"] = script_result["stdout"][:2000]
                                except Exception as script_exc:
                                    logger.warning("Script execution failed: %s — %s", script.name, script_exc)
                except Exception as extract_exc:
                    logger.warning("Failed to extract scripts: %s", extract_exc)

                # Write to shared memory
                shared.entries[agent_id] = json.dumps(agent.memory.to_dict())
                shared.globals[step.step_id] = result.text[:1000]

                # Broadcast memory update
                await ws_manager.broadcast(
                    channel,
                    "agent.memory.update",
                    {
                        "runId": run_id,
                        "agentId": agent_id,
                        "stepId": step.step_id,
                        "memory": agent.memory.to_dict(),
                        "sharedMemory": shared.to_dict(),
                    },
                )

                # Log step complete
                workflow_logger.log_run_event(
                    self._runs[run_id]["workflowId"],
                    run_id,
                    "agent.step.complete",
                    {"agentId": agent_id, "stepId": step.step_id, "tokensUsed": result.tokens_used, "durationMs": result.duration_ms},
                )

                # Collect system stats for this step
                step_metrics = {"cpuPercent": 0, "gpuPercent": 0, "memoryMb": 0}
                try:
                    from engine.services.system_stats import get_system_stats

                    stats = get_system_stats()
                    step_metrics["cpuPercent"] = stats.get("cpu_percent", 0)
                    step_metrics["gpuPercent"] = stats.get("gpu_percent", 0)
                    step_metrics["memoryMb"] = stats.get("memory_used_mb", 0)
                except Exception:
                    pass

                # Broadcast step complete with full metrics
                await ws_manager.broadcast(
                    channel,
                    "agent.step.complete",
                    {
                        "runId": run_id,
                        "agentId": agent_id,
                        "stepId": step.step_id,
                        "output": result.text,
                        "tokensUsed": result.tokens_used,
                        "durationMs": result.duration_ms,
                        "cpuPercent": step_metrics["cpuPercent"],
                        "gpuPercent": step_metrics["gpuPercent"],
                        "memoryMb": step_metrics["memoryMb"],
                        "model": step.local_model.get("model", "") if step.local_model else "",
                        "engine": step.local_model.get("engine", "") if step.local_model else "",
                    },
                )

                # Audit: step complete
                execution_audit.log_step_complete(
                    run_id,
                    agent_id,
                    step.step_id,
                    result.tokens_used,
                    result.duration_ms,
                )

                # Persist step status: completed
                await self._persist_step_status(
                    run_id,
                    step,
                    agent_id,
                    "completed",
                    tokens_used=result.tokens_used,
                    duration_ms=result.duration_ms,
                    cpu_percent=step_metrics["cpuPercent"],
                    gpu_percent=step_metrics["gpuPercent"],
                    memory_mb=step_metrics["memoryMb"],
                    model=step.local_model.get("model", "") if step.local_model else "",
                    engine=step.local_model.get("engine", "") if step.local_model else "",
                    response_preview=result.text[:500],
                )

                # Persist enriched step log to frontend
                try:
                    _run = self._runs.get(run_id, {})
                    async with httpx.AsyncClient(timeout=5) as _client:
                        await _client.post(
                            "http://localhost:3000/api/logs",
                            json={
                                "level": "info",
                                "message": f"Step '{step.step_name or step.step_id}' completed",
                                "source": "workflow",
                                "runId": run_id,
                                "metadata": {
                                    "workflowId": _run.get("workflowId", ""),
                                    "workflowName": _run.get("name", ""),
                                    "expertId": step.expert_id or "",
                                    "expertName": expert_data.get("name", "") if expert_data else "",
                                    "stepId": step.step_id,
                                    "stepName": step.step_name or step.step_id,
                                    "agentId": agent_id,
                                    "tokensUsed": result.tokens_used,
                                    "durationMs": result.duration_ms,
                                },
                            },
                        )
                except Exception:
                    pass  # Non-critical

                return agent

            except Exception as exc:
                # Try fallback model for local inference
                if settings.agent_retry_enabled and step.model_source == "local" and step.local_model and not getattr(step, "_is_fallback", False):
                    engine = step.local_model.get("engine", "ollama")
                    fallback_model = self.FALLBACK_MODELS.get(engine)
                    if fallback_model:
                        logger.warning(
                            "Agent %s failed with primary model, trying fallback %s: %s",
                            agent_id,
                            fallback_model,
                            exc,
                        )
                        fallback_step = StepConfig(
                            step_id=step.step_id,
                            expert_id=step.expert_id,
                            task_description=step.task_description,
                            model_source="local",
                            local_model={**step.local_model, "model": fallback_model},
                            temperature=step.temperature,
                            max_tokens=min(step.max_tokens, 2048),  # reduce for smaller model
                            connection_type=step.connection_type,
                        )
                        fallback_step._is_fallback = True  # type: ignore[attr-defined]
                        try:
                            result = await self._infer(fallback_step, system_prompt, user_prompt)
                            agent.output = f"[Fallback model: {fallback_model}]\n{result.text}"
                            agent.tokens_used = result.tokens_used
                            agent.duration_ms = result.duration_ms
                            agent.status = "completed"
                            agent.memory.findings.append(f"[Fallback] {result.text[:500]}")
                            shared.entries[agent_id] = json.dumps(agent.memory.to_dict())
                            shared.globals[step.step_id] = result.text[:1000]
                            await ws_manager.broadcast(
                                channel,
                                "agent.step.complete",
                                {
                                    "runId": run_id,
                                    "agentId": agent_id,
                                    "stepId": step.step_id,
                                    "output": agent.output,
                                    "tokensUsed": result.tokens_used,
                                    "durationMs": result.duration_ms,
                                    "fallback": True,
                                },
                            )
                            # Audit: fallback inference
                            execution_audit.log_inference(
                                run_id,
                                agent_id,
                                system_prompt,
                                user_prompt,
                                result.text,
                                result.tokens_used,
                                result.duration_ms,
                                model=fallback_model,
                                engine=engine,
                                temperature=step.temperature,
                                max_tokens=step.max_tokens,
                                fallback=True,
                            )
                            execution_audit.log_step_complete(
                                run_id,
                                agent_id,
                                step.step_id,
                                result.tokens_used,
                                result.duration_ms,
                            )
                            # Persist fallback step status: completed
                            await self._persist_step_status(
                                run_id,
                                step,
                                agent_id,
                                "completed",
                                tokens_used=result.tokens_used,
                                duration_ms=result.duration_ms,
                                model=fallback_model,
                                engine=engine,
                                response_preview=result.text[:500],
                            )
                            return agent
                        except Exception as fallback_exc:
                            logger.error("Fallback also failed for agent %s: %s", agent_id, fallback_exc)

                agent.status = "failed"
                agent.error = str(exc)

                # Persist failure artifacts
                try:
                    _step_name = step.step_name or step.step_id
                    step_artifacts.save_failure_log(
                        workflow_name=self._runs[run_id].get("name", "unnamed"),
                        step_name=_step_name,
                        run_id=run_id,
                        error=str(exc),
                        agent_id=agent_id,
                        phase="execute",
                    )
                except Exception:
                    pass  # Never let failure logging break the flow

                await ws_manager.broadcast(
                    channel,
                    "agent.step.failed",
                    {
                        "runId": run_id,
                        "agentId": agent_id,
                        "stepId": step.step_id,
                        "error": str(exc),
                    },
                )

                # Audit: step failed
                execution_audit.log_step_failed(run_id, agent_id, step.step_id, str(exc))

                # Persist step status: failed
                await self._persist_step_status(
                    run_id,
                    step,
                    agent_id,
                    "failed",
                    error_message=str(exc),
                )

                # Persist enriched step failure log to frontend
                try:
                    _run = self._runs.get(run_id, {})
                    async with httpx.AsyncClient(timeout=5) as _client:
                        await _client.post(
                            "http://localhost:3000/api/logs",
                            json={
                                "level": "error",
                                "message": f"Step '{step.step_name or step.step_id}' failed: {str(exc)[:200]}",
                                "source": "workflow",
                                "runId": run_id,
                                "metadata": {
                                    "workflowId": _run.get("workflowId", ""),
                                    "workflowName": _run.get("name", ""),
                                    "expertId": step.expert_id or "",
                                    "expertName": expert_data.get("name", "") if expert_data else "",
                                    "stepId": step.step_id,
                                    "stepName": step.step_name or step.step_id,
                                    "agentId": agent_id,
                                    "error": str(exc)[:500],
                                },
                            },
                        )
                except Exception:
                    pass  # Non-critical

                raise

    def _evaluate_condition(
        self,
        step: StepConfig,
        previous_output: str,
        run_id: str,
    ) -> bool:
        """Evaluate whether a conditional step should execute.

        Supports the following condition types via ``step.condition``:

        - ``{"type": "contains", "value": "<keyword>"}``
            Run only if *previous_output* contains *value*.
        - ``{"type": "not_contains", "value": "<keyword>"}``
            Skip if *previous_output* contains *value*.
        - ``{"type": "previous_succeeded"}``
            Run only if the most recent step completed successfully
            (i.e. no ``[FAILED]`` marker and non-empty output).
        - ``{"type": "previous_failed"}``
            Run only if the most recent step failed.
        - ``{"type": "always"}``
            Always run — effectively a no-op guard.

        When ``step.condition`` is *None* (legacy behaviour), falls back to
        the original heuristic: skip when *previous_output* is empty or
        contains ``[SKIP]``.
        """
        condition = step.condition

        # Legacy fallback: no explicit condition configured
        if condition is None:
            return bool(previous_output) and "[SKIP]" not in previous_output

        cond_type = condition.get("type", "always")

        if cond_type == "always":
            return True

        if cond_type == "contains":
            value = condition.get("value", "")
            return value != "" and value in previous_output

        if cond_type == "not_contains":
            value = condition.get("value", "")
            return value == "" or value not in previous_output

        if cond_type == "previous_succeeded":
            # Check that previous output exists and has no failure markers
            if not previous_output:
                return False
            step_results = self._runs.get(run_id, {}).get("stepResults", {})
            if step_results:
                last_result = list(step_results.values())[-1]
                if "[FAILED]" in str(last_result) or "[SKIPPED]" in str(last_result):
                    return False
            return True

        if cond_type == "previous_failed":
            if not previous_output:
                return True  # no output implies failure
            step_results = self._runs.get(run_id, {}).get("stepResults", {})
            if step_results:
                last_result = list(step_results.values())[-1]
                return "[FAILED]" in str(last_result)
            return False

        # Unknown condition type — log and default to running the step
        logger.warning(
            "Unknown condition type '%s' on step %s, defaulting to run",
            cond_type,
            step.step_id,
        )
        return True

    async def _resolve_expert(self, step: StepConfig) -> dict[str, Any] | None:
        """Resolve expert config from the frontend API if expertId is set."""
        if not step.expert_id:
            return None
        try:
            frontend_url = "http://localhost:3000"
            async with httpx.AsyncClient(timeout=10) as client:
                resp = await client.get(f"{frontend_url}/api/experts", params={"id": step.expert_id})
                if resp.status_code == 200:
                    data = resp.json()
                    return data.get("expert")
        except Exception as exc:
            logger.warning("Failed to resolve expert %s: %s", step.expert_id, exc)
        return None

    def _build_system_prompt(self, step: StepConfig, shared: SharedMemory, expert: dict[str, Any] | None = None) -> str:
        """Build the system prompt with role context, step instructions, and shared memory."""
        parts = []

        # Expert's system prompt
        if expert and expert.get("systemPrompt"):
            parts.append(expert["systemPrompt"])
            parts.append("")

        # Per-step system instructions override/extend
        if step.system_instructions:
            parts.append("## Step-Specific Instructions")
            parts.append(step.system_instructions)
            parts.append("")

        parts.extend(
            [
                "You are a specialized AI expert in a multi-agent workflow.",
                f"Your task: {step.task_description}",
            ]
        )
        if expert:
            parts.insert(len(parts) - 2, f"Role: {expert.get('role', 'agent')}")
            parts.insert(len(parts) - 2, f"Expert: {expert.get('name', 'unnamed')}")

        # Voice command
        if step.voice_command:
            parts.append("")
            parts.append("## Voice Command")
            parts.append(f'The user said: "{step.voice_command}"')
            parts.append("Incorporate this verbal instruction into your execution.")

        parts.extend(
            [
                "",
                "## Shared Context from Other Agents",
            ]
        )

        if shared.globals:
            for step_id, content in shared.globals.items():
                parts.append(f"### Output from step {step_id}:")
                parts.append(content[:500])
                parts.append("")
        else:
            parts.append("No previous agent outputs available yet.")

        parts.extend(
            [
                "",
                "## Instructions",
                "- Focus on your specific task description",
                "- Build upon findings from previous agents when available",
                "- Be thorough and precise in your output",
                "- Structure your response clearly",
            ]
        )

        return "\n".join(parts)

    async def _build_user_prompt(
        self,
        step: StepConfig,
        goal_content: str,
        input_contents: dict[str, str],
        previous_output: str,
        shared: SharedMemory,
    ) -> str:
        """Build the user prompt with goal, files, and previous context."""
        parts = [
            "## Goal",
            goal_content,
            "",
        ]

        if input_contents:
            parts.append("## Input Files")
            for filename, content in input_contents.items():
                parts.append(f"### {filename}")
                parts.append(content[:5000])
                parts.append("")

        if previous_output:
            parts.append("## Previous Step Output")
            parts.append(previous_output[:3000])
            parts.append("")

        # Step-specific file locations
        if step.file_locations:
            parts.append("## Referenced File Locations")
            for loc in step.file_locations:
                content = await self._read_file(loc)
                parts.append(f"### {loc}")
                parts.append(content[:3000])
                parts.append("")

        # Step-specific attached files/images
        if step.step_file_names or step.step_image_names:
            parts.append("## Step Attachments")
            for fname in step.step_file_names:
                content = await self._read_file(fname)
                parts.append(f"### {fname}")
                parts.append(content[:3000])
                parts.append("")
            for fname in step.step_image_names:
                parts.append(f"[Image attached: {fname}]")
            parts.append("")

        parts.extend(
            [
                "## Your Task",
                step.task_description,
            ]
        )

        return "\n".join(parts)

    async def _infer(
        self,
        step: StepConfig,
        system_prompt: str,
        user_prompt: str,
    ) -> GenerateResult:
        """Route inference to the correct backend with model pool tracking.

        When auto_route_by_connection_type is enabled (default):
        - Sequential steps → Ollama (reliable single-stream inference)
        - Parallel steps → llama.cpp (designed for concurrent requests)
        Explicit local_model.engine settings always take precedence.
        """
        if step.model_source == "local" and step.local_model:
            engine = step.local_model.get("engine", settings.default_local_engine)
            model = step.local_model.get("model", settings.default_local_model)
            base_url = step.local_model.get("baseUrl")

            # Auto-route by connection type if no explicit engine was set
            if settings.auto_route_by_connection_type and "engine" not in step.local_model:
                if step.connection_type == "parallel" and settings.llamacpp_available:
                    engine = "llamacpp"
                    base_url = base_url or settings.llamacpp_url
                    logger.info("Auto-routing parallel step %s to llama.cpp", step.step_id)
                else:
                    engine = "ollama"
                    if step.connection_type == "parallel":
                        logger.warning(
                            "llama.cpp not available — falling back to Ollama for parallel step %s",
                            step.step_id,
                        )
                    else:
                        logger.info("Auto-routing sequential step %s to Ollama", step.step_id)

            messages = [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ]

            await model_pool.acquire(model)
            try:
                return await inference_router.chat(
                    engine=engine,
                    model=model,
                    messages=messages,
                    temperature=step.temperature,
                    max_tokens=step.max_tokens,
                    base_url=base_url,
                )
            finally:
                await model_pool.release(model)

        # Provider-based: use HuggingFace inference as fallback
        from engine.services.hf import hf_service

        combined = f"{system_prompt}\n\n{user_prompt}"
        text = hf_service.text_generation(
            model_id="google/flan-t5-base",
            prompt=combined,
            max_new_tokens=step.max_tokens,
            temperature=step.temperature,
        )
        return GenerateResult(
            text=str(text),
            tokens_used=len(combined.split()) + len(str(text).split()),
            model="hf-fallback",
            duration_ms=0,
        )

    async def _persist_step_status(
        self,
        run_id: str,
        step: StepConfig,
        agent_id: str,
        status: str,
        *,
        tokens_used: int = 0,
        duration_ms: float = 0,
        cpu_percent: float = 0,
        gpu_percent: float = 0,
        memory_mb: float = 0,
        model: str = "",
        engine: str = "",
        response_preview: str = "",
        error_message: str = "",
    ) -> None:
        """Persist step execution status to NeonDB in real-time via frontend API."""
        payload: dict[str, Any] = {
            "runId": run_id,
            "workflowId": self._runs.get(run_id, {}).get("workflowId", ""),
            "stepId": step.step_id,
            "agentId": agent_id,
            "stepName": step.step_name or step.step_id,
            "expertId": step.expert_id or None,
            "status": status,
            "model": model or (step.local_model.get("model", "") if step.local_model else ""),
            "engine": engine or (step.local_model.get("engine", "") if step.local_model else ""),
            "startedAt": datetime.now(UTC).isoformat(),
        }
        if tokens_used:
            payload["tokensUsed"] = tokens_used
        if duration_ms:
            payload["durationMs"] = round(duration_ms)
        if cpu_percent:
            payload["cpuPercent"] = cpu_percent
        if gpu_percent:
            payload["gpuPercent"] = gpu_percent
        if memory_mb:
            payload["memoryMb"] = memory_mb
        if response_preview:
            payload["responsePreview"] = response_preview[:500]
        if error_message:
            payload["errorMessage"] = error_message
        if status in ("completed", "failed"):
            payload["completedAt"] = datetime.now(UTC).isoformat()

        try:
            async with httpx.AsyncClient(timeout=5) as client:
                await client.post(
                    "http://localhost:3000/api/workflows/executions",
                    json=payload,
                )
        except Exception as exc:
            logger.warning("Failed to persist step status: %s", exc)

    async def _read_file(self, url_or_path: str) -> str:
        """Read a file from the upload directory or a URL."""
        # Local file path
        path = Path(settings.upload_dir) / Path(url_or_path).name
        if path.exists():
            return path.read_text(encoding="utf-8")

        # Try as absolute path
        abs_path = Path(url_or_path)
        if abs_path.exists():
            return abs_path.read_text(encoding="utf-8")

        # Try as URL
        if url_or_path.startswith(("http://", "https://")):
            async with httpx.AsyncClient(timeout=30) as client:
                resp = await client.get(url_or_path)
                resp.raise_for_status()
                return resp.text

        return f"[Could not read file: {url_or_path}]"

    async def cancel_run(self, run_id: str) -> dict[str, Any]:
        """Cancel a running workflow by setting its cancellation event."""
        run = self._runs.get(run_id)
        if not run:
            return {"error": "Run not found", "runId": run_id}
        if run["status"] != "running":
            return {"error": f"Run is not running (status: {run['status']})", "runId": run_id}

        evt = self._cancellation_events.get(run_id)
        if evt:
            evt.set()
        return {"runId": run_id, "status": "cancelling", "message": "Cancel signal sent"}

    async def restart_run(self, run_id: str) -> dict[str, Any]:
        """Restart a completed/failed/cancelled workflow using its original request."""
        run = self._runs.get(run_id)
        if not run:
            return {"error": "Run not found", "runId": run_id}
        if run["status"] == "running":
            return {"error": "Run is still running", "runId": run_id}

        request: WorkflowRequest | None = run.get("request")
        if not request:
            return {"error": "Original request not stored for this run", "runId": run_id}

        result = await self.execute_workflow(request)
        return result

    async def _broadcast_live_metrics(
        self,
        run_id: str,
        agent_id: str,
        step_id: str,
        channel: str,
    ) -> None:
        """Broadcast system metrics every 2s while an agent is running."""
        stop_evt = self._metrics_stop_events.get(run_id)
        if not stop_evt:
            return
        while not stop_evt.is_set():
            try:
                from engine.services.system_stats import get_system_stats

                stats = get_system_stats()
                run = self._runs.get(run_id)
                if not run:
                    break
                started = run.get("startedAt", "")
                elapsed_ms = 0
                if started:
                    elapsed_ms = int((datetime.now(UTC) - datetime.fromisoformat(started)).total_seconds() * 1000)
                agent = run.get("agents", {}).get(agent_id)
                tokens = agent.tokens_used if agent else 0
                await ws_manager.broadcast(
                    channel,
                    "run.metrics.update",
                    {
                        "runId": run_id,
                        "agentId": agent_id,
                        "stepId": step_id,
                        "cpuPercent": stats.get("cpu_percent", 0),
                        "gpuPercent": stats.get("gpu_percent", 0),
                        "memoryMb": stats.get("memory_used_mb", 0),
                        "tokensUsed": tokens,
                        "elapsedMs": elapsed_ms,
                    },
                )
            except Exception:
                pass
            try:
                await asyncio.wait_for(asyncio.shield(stop_evt.wait()), timeout=2.0)
                break  # Event was set, stop broadcasting
            except TimeoutError:
                pass  # Continue broadcasting

    def get_run(self, run_id: str) -> dict[str, Any] | None:
        return self._runs.get(run_id)

    def get_shared_memory(self, run_id: str) -> SharedMemory | None:
        return self._shared_memory.get(run_id)

    def _compute_run_stats(self, run_id: str) -> tuple[int, int, int, int]:
        """Return (total_tokens, total_duration_ms, succeeded, failed) for a run."""
        agents = self._runs[run_id].get("agents", {})
        total_tokens = sum(a.tokens_used for a in agents.values())
        total_duration = sum(int(a.duration_ms) for a in agents.values())
        succeeded = sum(1 for a in agents.values() if a.status == "completed")
        failed = sum(1 for a in agents.values() if a.status == "failed")
        return total_tokens, total_duration, succeeded, failed

    async def _update_expert_stats(self, run_id: str) -> None:
        """Broadcast expert stats updates after run completion."""
        agents = self._runs[run_id].get("agents", {})
        for agent in agents.values():
            if agent.status == "completed":
                await ws_manager.broadcast(
                    f"workflow.{run_id}",
                    "expert.stats.update",
                    {
                        "runId": run_id,
                        "agentId": agent.agent_id,
                        "stepId": agent.step_id,
                        "tokensUsed": agent.tokens_used,
                        "durationMs": agent.duration_ms,
                        "status": agent.status,
                    },
                )

    async def _sync_to_frontend(
        self,
        run_id: str,
        workflow_id: str,
        workflow_name: str,
        status: str,
        steps: list[StepConfig],
        error_message: str | None = None,
    ) -> None:
        """Sync workflow run results and step execution metrics to the frontend DB."""
        run = self._runs.get(run_id)
        if not run:
            return

        started_at = run.get("startedAt", "")
        completed_at = run.get("completedAt", datetime.now(UTC).isoformat())
        total_tokens, total_duration_ms, _, _ = self._compute_run_stats(run_id)
        duration_sec = round(total_duration_ms / 1000, 3) if total_duration_ms else 0

        # Build expert chain from agents
        agents: dict[str, AgentState] = run.get("agents", {})
        expert_chain: list[str] = []
        for agent in agents.values():
            step_cfg = next((s for s in steps if s.step_id == agent.step_id), None)
            name = step_cfg.step_name if step_cfg and step_cfg.step_name else agent.step_id
            expert_chain.append(name)

        try:
            async with httpx.AsyncClient(timeout=10) as client:
                # 1. POST run status to frontend DB
                run_payload = {
                    "id": run_id,
                    "workflowId": workflow_id,
                    "workflowName": workflow_name,
                    "status": status,
                    "startedAt": started_at,
                    "completedAt": completed_at,
                    "totalTokensUsed": total_tokens,
                    "durationSec": duration_sec,
                    "expertChain": expert_chain,
                }
                if error_message:
                    run_payload["errorMessage"] = error_message

                await client.post(
                    "http://localhost:3000/api/workflows/runs",
                    json=run_payload,
                )

                # 2. POST step execution metrics for each agent
                for agent in agents.values():
                    step_cfg = next((s for s in steps if s.step_id == agent.step_id), None)
                    model = ""
                    engine = ""
                    if step_cfg and step_cfg.local_model:
                        model = step_cfg.local_model.get("model", "")
                        engine = step_cfg.local_model.get("engine", "")

                    # Collect system metrics snapshot
                    cpu_percent = 0.0
                    gpu_percent = 0.0
                    memory_mb = 0.0
                    try:
                        from engine.services.system_stats import get_system_stats

                        stats = get_system_stats()
                        cpu_percent = stats.get("cpu_percent", 0)
                        gpu_percent = stats.get("gpu_percent", 0)
                        memory_mb = stats.get("memory_used_mb", 0)
                    except Exception:
                        pass

                    step_payload = {
                        "runId": run_id,
                        "stepId": agent.step_id,
                        "agentId": agent.agent_id,
                        "expertId": step_cfg.expert_id if step_cfg else None,
                        "stepName": (step_cfg.step_name if step_cfg and step_cfg.step_name else agent.step_id),
                        "status": agent.status,
                        "model": model,
                        "engine": engine,
                        "tokensUsed": agent.tokens_used,
                        "durationMs": agent.duration_ms,
                        "cpuPercent": cpu_percent,
                        "gpuPercent": gpu_percent,
                        "memoryMb": memory_mb,
                        "promptPreview": (agent.memory.plan[:500] if agent.memory.plan else ""),
                        "responsePreview": (agent.output[:500] if agent.output else ""),
                    }

                    await client.post(
                        "http://localhost:3000/api/workflows/executions",
                        json=step_payload,
                    )

                # 3. Register step artifacts as assets in frontend DB
                for agent in agents.values():
                    step_cfg = next((s for s in steps if s.step_id == agent.step_id), None)
                    _step_name = step_cfg.step_name if step_cfg and step_cfg.step_name else agent.step_id
                    artifacts = step_artifacts.list_artifacts(workflow_name, _step_name)
                    if artifacts:
                        step_dir = step_artifacts.get_step_dir(workflow_name, _step_name)
                        asset_records = []
                        for a in artifacts:
                            file_path = str(step_dir / a["name"])
                            ext = a.get("type", "").lower()
                            mime = "text/plain"
                            file_type = "document"
                            if ext in (".json",):
                                mime = "application/json"
                            elif ext in (".py", ".js", ".ts", ".sh"):
                                file_type = "file"
                            asset_records.append(
                                {
                                    "name": a["name"],
                                    "fileName": a["name"],
                                    "filePath": file_path,
                                    "sizeBytes": a.get("size", 0),
                                    "mimeType": mime,
                                    "fileType": file_type,
                                    "sourceType": "workflow",
                                    "folder": f"/workflows/{workflow_name}/{_step_name}",
                                    "tags": [
                                        "workflow",
                                        workflow_name,
                                        _step_name,
                                    ],
                                    "metadata": {
                                        "runId": run_id,
                                        "workflowId": workflow_id,
                                        "workflowName": workflow_name,
                                        "stepName": _step_name,
                                    },
                                }
                            )
                        if asset_records:
                            await client.post(
                                "http://localhost:3000/api/assets/register",
                                json={"assets": asset_records},
                            )
        except Exception as sync_exc:
            logger.warning("Failed to sync run %s to frontend DB: %s", run_id, sync_exc)

    def get_status(self) -> dict[str, Any]:
        """Return orchestrator status for monitoring."""
        active_runs = {rid: r["status"] for rid, r in self._runs.items() if r["status"] == "running"}
        return {
            "active_runs": len(active_runs),
            "total_runs": len(self._runs),
            "runs": active_runs,
            "semaphore_available": self._semaphore._value,
            "max_concurrent_agents": settings.max_concurrent_agents,
            "active_models": model_pool.active_models,
            "total_active_inferences": model_pool.total_active,
        }


orchestrator = AgentOrchestrator()
