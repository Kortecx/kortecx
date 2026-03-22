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
from engine.services.execution_audit import execution_audit
from engine.services.local_inference import GenerateResult, inference_router, model_pool
from engine.services.step_artifacts import step_artifacts
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


@dataclass
class WorkflowRequest:
    workflow_id: str
    name: str
    goal_file_url: str
    input_file_urls: list[str]
    steps: list[StepConfig]


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

    async def execute_workflow(self, request: WorkflowRequest) -> dict[str, Any]:
        """Main entry point — creates a run and orchestrates agents."""
        run_id = f"run-{uuid.uuid4().hex[:12]}"
        channel = f"workflow.{run_id}"

        shared = SharedMemory(run_id=run_id)
        self._shared_memory[run_id] = shared

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
                            "source": "orchestrator",
                            "runId": run_id,
                            "metadata": {
                                "workflowId": request.workflow_id,
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
                            "source": "orchestrator",
                            "runId": run_id,
                            "metadata": {
                                "workflowId": request.workflow_id,
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
                tasks = [
                    self._run_agent(
                        run_id,
                        step,
                        shared,
                        goal_content,
                        input_contents,
                        previous_output,
                        channel,
                    )
                    for step in group
                ]
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

                # Execute inference
                result = await self._infer(step, system_prompt, user_prompt)

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
        """Route inference to the correct backend with model pool tracking."""
        if step.model_source == "local" and step.local_model:
            engine = step.local_model.get("engine", settings.default_local_engine)
            model = step.local_model.get("model", settings.default_local_model)
            base_url = step.local_model.get("baseUrl")

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
            name = (step_cfg.step_name if step_cfg and step_cfg.step_name else agent.step_id)
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
