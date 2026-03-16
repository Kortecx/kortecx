"""Agent orchestrator — spawns agents, manages shared memory, coordinates execution."""

from __future__ import annotations

import asyncio
import logging
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from engine.config import settings
from engine.core.websocket import ws_manager
from engine.services.local_inference import GenerateResult, inference_router
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
    entries: dict[str, str] = field(default_factory=dict)   # agentId -> JSON
    globals: dict[str, str] = field(default_factory=dict)   # shared KV

    def to_dict(self) -> dict[str, Any]:
        return {"runId": self.run_id, "entries": self.entries, "globals": self.globals}


@dataclass
class StepIntegration:
    id: str
    type: str                    # "integration" | "plugin"
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
    model_source: str            # "local" | "provider"
    local_model: dict[str, Any] | None
    temperature: float
    max_tokens: int
    connection_type: str         # "sequential" | "parallel"
    system_instructions: str = ""
    voice_command: str = ""
    file_locations: list[str] = field(default_factory=list)
    step_file_names: list[str] = field(default_factory=list)
    step_image_names: list[str] = field(default_factory=list)
    integrations: list[StepIntegration] = field(default_factory=list)


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
    status: str = "pending"      # pending | running | completed | failed
    memory: AgentMemory = field(default_factory=AgentMemory)
    output: str = ""
    error: str = ""
    tokens_used: int = 0
    duration_ms: float = 0


class AgentOrchestrator:
    """Core orchestration runtime for workflow agent execution."""

    def __init__(self) -> None:
        self._runs: dict[str, dict[str, Any]] = {}   # runId -> run state
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
            "startedAt": datetime.now(timezone.utc).isoformat(),
            "agents": {},
            "stepResults": {},
        }

        # Log run start
        workflow_logger.log_run_event(request.workflow_id, run_id, "run.started", {
            "name": request.name, "totalSteps": len(request.steps),
        })

        # Broadcast run started
        await ws_manager.broadcast(channel, "run.started", {
            "runId": run_id,
            "name": request.name,
            "totalSteps": len(request.steps),
        })

        # Orchestrate
        try:
            result = await self._orchestrate(
                run_id, request.steps, shared,
                goal_content, input_contents, channel,
            )
            self._runs[run_id]["status"] = "completed"
            self._runs[run_id]["completedAt"] = datetime.now(timezone.utc).isoformat()

            # Log completion
            workflow_logger.log_run_event(request.workflow_id, run_id, "run.completed", {
                "outputLength": len(result),
            })
            workflow_logger.save_run_memory(request.workflow_id, run_id, shared.to_dict())
            workflow_logger.save_run_output(request.workflow_id, run_id, result)

            await ws_manager.broadcast(channel, "workflow.complete", {
                "runId": run_id,
                "output": result,
                "sharedMemory": shared.to_dict(),
            })

            return {"runId": run_id, "status": "completed", "output": result}

        except Exception as exc:
            logger.exception("Workflow %s failed", run_id)
            self._runs[run_id]["status"] = "failed"
            self._runs[run_id]["error"] = str(exc)

            workflow_logger.log_run_event(request.workflow_id, run_id, "run.failed", {
                "error": str(exc),
            })

            await ws_manager.broadcast(channel, "workflow.failed", {
                "runId": run_id,
                "error": str(exc),
            })

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
        """Execute steps respecting connection types (sequential / parallel)."""
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
                # Sequential execution
                step = group[0]
                agent_state = await self._run_agent(
                    run_id, step, shared, goal_content,
                    input_contents, previous_output, channel,
                )
                previous_output = agent_state.output
                self._runs[run_id]["stepResults"][step.step_id] = agent_state.output
            else:
                # Parallel execution
                tasks = [
                    self._run_agent(
                        run_id, step, shared, goal_content,
                        input_contents, previous_output, channel,
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
        agent.memory.plan = (
            f"Step: {step.task_description}\n"
            f"Goal:\n{goal_content[:2000]}"
        )

        self._runs[run_id]["agents"][agent_id] = agent

        # Log agent spawn
        workflow_logger.log_run_event(
            self._runs[run_id]["workflowId"], run_id, "agent.spawned",
            {"agentId": agent_id, "stepId": step.step_id, "modelSource": step.model_source},
        )

        # Broadcast agent spawned
        await ws_manager.broadcast(channel, "agent.spawned", {
            "runId": run_id,
            "agentId": agent_id,
            "stepId": step.step_id,
            "taskDescription": step.task_description,
            "modelSource": step.model_source,
        })

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
                    step, goal_content, input_contents, previous_output, shared,
                )

                # Broadcast thinking
                await ws_manager.broadcast(channel, "agent.thinking", {
                    "runId": run_id,
                    "agentId": agent_id,
                    "stepId": step.step_id,
                })

                # Execute inference
                result = await self._infer(step, system_prompt, user_prompt)

                agent.output = result.text
                agent.tokens_used = result.tokens_used
                agent.duration_ms = result.duration_ms
                agent.status = "completed"

                # Update agent memory with findings
                agent.memory.findings.append(result.text[:500])

                # Write to shared memory
                import json
                shared.entries[agent_id] = json.dumps(agent.memory.to_dict())
                shared.globals[step.step_id] = result.text[:1000]

                # Broadcast memory update
                await ws_manager.broadcast(channel, "agent.memory.update", {
                    "runId": run_id,
                    "agentId": agent_id,
                    "stepId": step.step_id,
                    "memory": agent.memory.to_dict(),
                    "sharedMemory": shared.to_dict(),
                })

                # Log step complete
                workflow_logger.log_run_event(
                    self._runs[run_id]["workflowId"], run_id, "agent.step.complete",
                    {"agentId": agent_id, "stepId": step.step_id, "tokensUsed": result.tokens_used, "durationMs": result.duration_ms},
                )

                # Broadcast step complete
                await ws_manager.broadcast(channel, "agent.step.complete", {
                    "runId": run_id,
                    "agentId": agent_id,
                    "stepId": step.step_id,
                    "output": result.text,
                    "tokensUsed": result.tokens_used,
                    "durationMs": result.duration_ms,
                })

                return agent

            except Exception as exc:
                agent.status = "failed"
                agent.error = str(exc)

                await ws_manager.broadcast(channel, "agent.step.failed", {
                    "runId": run_id,
                    "agentId": agent_id,
                    "stepId": step.step_id,
                    "error": str(exc),
                })

                raise

    async def _resolve_expert(self, step: StepConfig) -> dict[str, Any] | None:
        """Resolve expert config from the frontend API if expertId is set."""
        if not step.expert_id:
            return None
        try:
            import httpx
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

        parts.extend([
            "You are a specialized AI expert in a multi-agent workflow.",
            f"Your task: {step.task_description}",
        ])
        if expert:
            parts.insert(len(parts) - 2, f"Role: {expert.get('role', 'agent')}")
            parts.insert(len(parts) - 2, f"Expert: {expert.get('name', 'unnamed')}")

        # Voice command
        if step.voice_command:
            parts.append("")
            parts.append("## Voice Command")
            parts.append(f'The user said: "{step.voice_command}"')
            parts.append("Incorporate this verbal instruction into your execution.")

        parts.extend([
            "",
            "## Shared Context from Other Agents",
        ])

        if shared.globals:
            for step_id, content in shared.globals.items():
                parts.append(f"### Output from step {step_id}:")
                parts.append(content[:500])
                parts.append("")
        else:
            parts.append("No previous agent outputs available yet.")

        parts.extend([
            "",
            "## Instructions",
            "- Focus on your specific task description",
            "- Build upon findings from previous agents when available",
            "- Be thorough and precise in your output",
            "- Structure your response clearly",
        ])

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

        parts.extend([
            "## Your Task",
            step.task_description,
        ])

        return "\n".join(parts)

    async def _infer(
        self,
        step: StepConfig,
        system_prompt: str,
        user_prompt: str,
    ) -> GenerateResult:
        """Route inference to the correct backend."""
        if step.model_source == "local" and step.local_model:
            engine = step.local_model.get("engine", "ollama")
            model = step.local_model.get("model", "llama3.1:8b")
            base_url = step.local_model.get("baseUrl")

            messages = [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ]

            return await inference_router.chat(
                engine=engine,
                model=model,
                messages=messages,
                temperature=step.temperature,
                max_tokens=step.max_tokens,
                base_url=base_url,
            )

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
            import httpx
            async with httpx.AsyncClient(timeout=30) as client:
                resp = await client.get(url_or_path)
                resp.raise_for_status()
                return resp.text

        return f"[Could not read file: {url_or_path}]"

    def get_run(self, run_id: str) -> dict[str, Any] | None:
        return self._runs.get(run_id)

    def get_shared_memory(self, run_id: str) -> SharedMemory | None:
        return self._shared_memory.get(run_id)


orchestrator = AgentOrchestrator()
