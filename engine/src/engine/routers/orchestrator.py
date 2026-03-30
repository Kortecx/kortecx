"""Orchestrator API — workflow execution, file uploads, run status."""

from __future__ import annotations

import json
import logging
import uuid
from pathlib import Path
from typing import Any

import httpx
from fastapi import APIRouter, BackgroundTasks, File, UploadFile
from fastapi.responses import StreamingResponse
from pydantic import BaseModel

from engine.config import settings
from engine.services.local_inference import OllamaService, inference_router, model_pool
from engine.services.orchestrator import (
    StepConfig,
    StepIntegration,
    WorkflowRequest,
    orchestrator,
)
from engine.services.step_artifacts import step_artifacts
from engine.services.workflow_artifacts import workflow_artifacts

logger = logging.getLogger("engine.routers.orchestrator")

router = APIRouter()


# ── Request / Response models ────────────────────────────────────────────────


class LocalModelConfigModel(BaseModel):
    engine: str = "ollama"  # ollama | llamacpp
    model: str = "llama3.1:8b"
    baseUrl: str | None = None


class StepIntegrationModel(BaseModel):
    id: str
    type: str  # integration | plugin
    referenceId: str
    name: str
    icon: str = ""
    color: str = ""
    config: dict[str, str] = {}


class StepConfigModel(BaseModel):
    stepId: str
    name: str = ""
    expertId: str | None = None
    taskDescription: str
    systemInstructions: str = ""
    voiceCommand: str = ""
    fileLocations: list[str] = []
    stepFileNames: list[str] = []
    stepImageNames: list[str] = []
    modelSource: str = "local"  # local | provider
    localModel: LocalModelConfigModel | None = None
    temperature: float = 0.7
    maxTokens: int = 4096
    connectionType: str = "sequential"  # sequential | parallel
    integrations: list[StepIntegrationModel] = []
    stepType: str = "agent"  # agent | action
    actionConfig: dict[str, Any] | None = None


class ExecuteRequest(BaseModel):
    runId: str | None = None
    workflowId: str | None = None
    name: str
    goalFileUrl: str
    inputFileUrls: list[str] = []
    steps: list[StepConfigModel]
    masterAgentId: str | None = None
    connectedAgentIds: list[str] = []
    failFast: bool = False


class ExecuteResponse(BaseModel):
    runId: str
    status: str
    message: str


# ── Routes ───────────────────────────────────────────────────────────────────


@router.post("/execute", response_model=ExecuteResponse)
async def execute_workflow(req: ExecuteRequest, bg: BackgroundTasks) -> ExecuteResponse:
    """Start a workflow execution — agents are spawned in the background."""
    if not req.name.strip():
        raise ValueError("Workflow name is required")
    if not req.steps:
        raise ValueError("At least one step is required")

    request = WorkflowRequest(
        workflow_id=req.workflowId or f"wf-{uuid.uuid4().hex[:8]}",
        name=req.name,
        goal_file_url=req.goalFileUrl,
        input_file_urls=req.inputFileUrls,
        master_agent_id=req.masterAgentId,
        connected_agent_ids=req.connectedAgentIds,
        fail_fast=req.failFast,
        steps=[
            StepConfig(
                step_id=s.stepId,
                expert_id=s.expertId,
                task_description=s.taskDescription,
                step_name=s.name or s.stepId,
                model_source=s.modelSource,
                local_model=s.localModel.model_dump() if s.localModel else None,
                temperature=s.temperature,
                max_tokens=s.maxTokens,
                connection_type=s.connectionType,
                system_instructions=s.systemInstructions,
                voice_command=s.voiceCommand,
                file_locations=s.fileLocations,
                step_file_names=s.stepFileNames,
                step_image_names=s.stepImageNames,
                integrations=[
                    StepIntegration(
                        id=si.id,
                        type=si.type,
                        reference_id=si.referenceId,
                        name=si.name,
                        icon=si.icon,
                        color=si.color,
                        config=si.config,
                    )
                    for si in s.integrations
                ],
                step_type=s.stepType,
                action_config=s.actionConfig,
            )
            for s in req.steps
        ],
    )

    # Run orchestration in background so we return immediately
    bg.add_task(orchestrator.execute_workflow, request, req.runId)

    return ExecuteResponse(
        runId=req.runId or request.workflow_id,
        status="started",
        message=f"Workflow '{req.name}' execution started — {len(req.steps)} agent(s) will be spawned",
    )


@router.post("/runs/{run_id}/cancel")
async def cancel_run(run_id: str) -> dict[str, Any]:
    """Cancel a running workflow execution."""
    return await orchestrator.cancel_run(run_id)


@router.post("/runs/{run_id}/restart")
async def restart_run(run_id: str, bg: BackgroundTasks) -> dict[str, Any]:
    """Restart a completed/failed/cancelled workflow using its original config."""
    run = orchestrator.get_run(run_id)
    if not run:
        return {"error": "Run not found", "runId": run_id}
    if run["status"] == "running":
        return {"error": "Run is still running", "runId": run_id}
    request = run.get("request")
    if not request:
        return {"error": "Original request not stored", "runId": run_id}
    bg.add_task(orchestrator.execute_workflow, request)
    return {"runId": run_id, "status": "restarting", "message": "Workflow restart initiated"}


@router.get("/runs/{run_id}")
async def get_run(run_id: str) -> dict[str, Any]:
    """Get the current status of a workflow run."""
    run = orchestrator.get_run(run_id)
    if not run:
        return {"error": "Run not found", "runId": run_id}
    # Serialize agent states
    serialized = {**run}
    serialized["agents"] = {
        aid: {
            "agentId": a.agent_id,
            "stepId": a.step_id,
            "status": a.status,
            "tokensUsed": a.tokens_used,
            "durationMs": a.duration_ms,
            "output": a.output[:500] if a.output else "",
            "error": a.error,
        }
        for aid, a in run.get("agents", {}).items()
    }
    return serialized


@router.get("/runs/{run_id}/memory")
async def get_shared_memory(run_id: str) -> dict[str, Any]:
    """Get the shared memory snapshot for a run."""
    mem = orchestrator.get_shared_memory(run_id)
    if not mem:
        return {"error": "Run not found", "runId": run_id}
    return mem.to_dict()


@router.get("/runs/{run_id}/artifacts")
async def list_run_artifacts(run_id: str) -> dict[str, Any]:
    """List all step artifacts for a workflow run."""
    run = orchestrator.get_run(run_id)
    if not run:
        return {"error": "Run not found", "runId": run_id, "artifacts": []}

    workflow_name = run.get("name", "unnamed")
    all_artifacts: list[dict[str, Any]] = []

    # Collect artifacts from all steps
    agents = run.get("agents", {})
    for agent in agents.values():
        step_name = getattr(agent, "step_id", "") or ""
        if step_name:
            artifacts = step_artifacts.list_artifacts(workflow_name, step_name)
            for a in artifacts:
                a["workflowName"] = workflow_name
                a["stepName"] = step_name
                a["runId"] = run_id
                a["sourceType"] = "workflow"
            all_artifacts.extend(artifacts)

    return {"artifacts": all_artifacts, "total": len(all_artifacts), "runId": run_id}


@router.post("/upload")
async def upload_files(files: list[UploadFile] = File(...)) -> dict[str, Any]:
    """Upload goal markdown and input files for workflow execution."""
    upload_dir = Path(settings.upload_dir)
    upload_dir.mkdir(parents=True, exist_ok=True)

    uploaded: list[dict[str, str]] = []
    for f in files:
        ext = Path(f.filename or "file").suffix
        file_id = uuid.uuid4().hex[:12]
        filename = f"{file_id}{ext}"
        path = upload_dir / filename

        content = await f.read()
        path.write_bytes(content)

        uploaded.append(
            {
                "id": file_id,
                "filename": f.filename or filename,
                "storedAs": filename,
                "url": f"/uploads/{filename}",
                "size": len(content),
            }
        )
        logger.info("Uploaded: %s → %s (%d bytes)", f.filename, filename, len(content))

    return {"files": uploaded, "count": len(uploaded)}


# ── Monitoring endpoints ──────────────────────────────────────────────────────


@router.get("/status")
async def orchestrator_status() -> dict[str, Any]:
    """Get orchestrator runtime status including system resources."""
    from engine.services.system_stats import get_process_stats, get_system_stats

    status = orchestrator.get_status()
    status["system"] = get_system_stats()
    status["process"] = get_process_stats()
    return status


@router.get("/system/stats")
async def system_stats() -> dict[str, Any]:
    """Get current CPU/GPU/memory usage — lightweight, for frequent polling."""
    from engine.services.system_stats import get_system_stats

    return get_system_stats()


@router.get("/models/active")
async def active_models() -> dict[str, Any]:
    """Get currently active model usage."""
    return {"models": model_pool.active_models, "total": model_pool.total_active}


@router.get("/models/{engine}/{model_name:path}/info")
async def model_info(engine: str, model_name: str) -> dict[str, Any]:
    """Get detailed model info (Ollama only)."""
    if engine != "ollama":
        return {"error": "Model info only supported for Ollama"}
    backend = inference_router.get_backend(engine)
    if not isinstance(backend, OllamaService):
        return {"error": "Not an Ollama backend"}
    return await backend.get_model_info(model_name)


# ── Local inference endpoints ────────────────────────────────────────────────


@router.get("/models/{engine}")
async def list_local_models(engine: str) -> dict[str, Any]:
    """List available models on a local inference engine."""
    try:
        models = await inference_router.list_models(engine)
        return {"engine": engine, "models": models}
    except Exception as exc:
        return {"engine": engine, "models": [], "error": str(exc)}


@router.get("/health/{engine}")
async def check_engine_health(engine: str) -> dict[str, Any]:
    """Check if a local inference engine is running."""
    healthy = await inference_router.health_check(engine)
    return {"engine": engine, "healthy": healthy}


@router.get("/capabilities")
async def get_capabilities() -> dict[str, Any]:
    """Return live capability flags (llama.cpp / Ollama availability)."""
    ollama_ok = await inference_router.health_check("ollama")
    llamacpp_ok = await inference_router.health_check("llamacpp")
    return {
        "ollama_available": ollama_ok,
        "llamacpp_available": llamacpp_ok,
    }


class PullModelRequest(BaseModel):
    engine: str = "ollama"
    model: str
    baseUrl: str | None = None


@router.post("/models/pull")
async def pull_model(req: PullModelRequest, bg: BackgroundTasks) -> dict[str, Any]:
    """Pull/download a model on Ollama. Runs in background."""
    if req.engine != "ollama":
        return {"error": "Model pull is only supported on Ollama"}

    from engine.services.local_inference import OllamaService

    svc = OllamaService(req.baseUrl) if req.baseUrl else OllamaService()

    async def _pull() -> None:
        try:
            await svc.pull_model(req.model)
            logger.info("Model pulled: %s", req.model)
        except Exception as exc:
            logger.error("Model pull failed: %s — %s", req.model, exc)

    bg.add_task(_pull)
    return {"status": "pulling", "engine": req.engine, "model": req.model}


@router.post("/models/pull/stream")
async def pull_model_stream(req: PullModelRequest):
    """Pull/download a model on Ollama with streaming progress via SSE."""
    if req.engine != "ollama":
        return {"error": "Model pull is only supported on Ollama"}

    base_url = (req.baseUrl or settings.ollama_url).rstrip("/")

    async def stream_progress():
        async with httpx.AsyncClient(base_url=base_url, timeout=600) as client:
            async with client.stream("POST", "/api/pull", json={"name": req.model, "stream": True}) as resp:
                resp.raise_for_status()
                async for line in resp.aiter_lines():
                    if not line.strip():
                        continue
                    try:
                        data = json.loads(line)
                        # Ollama sends: {"status": "pulling ...", "digest": "...", "total": N, "completed": N}
                        event = {
                            "status": data.get("status", ""),
                            "digest": data.get("digest", ""),
                            "total": data.get("total", 0),
                            "completed": data.get("completed", 0),
                        }
                        if event["total"] > 0:
                            event["percent"] = round((event["completed"] / event["total"]) * 100, 1)
                        yield f"data: {json.dumps(event)}\n\n"
                    except Exception:
                        continue
        yield f"data: {json.dumps({'status': 'success', 'percent': 100})}\n\n"

    return StreamingResponse(stream_progress(), media_type="text/event-stream")


class DeleteModelRequest(BaseModel):
    engine: str = "ollama"
    model: str
    baseUrl: str | None = None


@router.post("/models/delete")
async def delete_model(req: DeleteModelRequest) -> dict[str, Any]:
    """Delete a local model from Ollama."""
    if req.engine != "ollama":
        return {"error": "Model delete is only supported on Ollama"}
    try:
        svc = OllamaService(req.baseUrl) if req.baseUrl else OllamaService()
        await svc.delete_model(req.model)
        return {"deleted": True, "model": req.model, "engine": req.engine}
    except Exception as exc:
        logger.error("Model delete failed: %s — %s", req.model, exc)
        return {"error": str(exc), "model": req.model}


class GenerateRequest(BaseModel):
    engine: str = "ollama"
    model: str
    prompt: str
    system: str = ""
    temperature: float = 0.7
    maxTokens: int = 4096
    baseUrl: str | None = None


class ChatRequest(BaseModel):
    engine: str = "ollama"
    model: str
    messages: list[dict[str, str]]
    temperature: float = 0.7
    maxTokens: int = 4096
    baseUrl: str | None = None


@router.post("/inference/generate")
async def local_generate(req: GenerateRequest) -> dict[str, Any]:
    """Run text generation on a local inference engine."""
    try:
        result = await inference_router.generate(
            engine=req.engine,
            model=req.model,
            prompt=req.prompt,
            system=req.system,
            temperature=req.temperature,
            max_tokens=req.maxTokens,
            base_url=req.baseUrl,
        )
        return {
            "text": result.text,
            "tokensUsed": result.tokens_used,
            "model": result.model,
            "durationMs": result.duration_ms,
        }
    except Exception as exc:
        logger.error("Local generate failed: %s", exc)
        return {"error": str(exc), "engine": req.engine, "model": req.model}


@router.post("/inference/chat")
async def local_chat(req: ChatRequest) -> dict[str, Any]:
    """Run chat completion on a local inference engine."""
    try:
        result = await inference_router.chat(
            engine=req.engine,
            model=req.model,
            messages=req.messages,
            temperature=req.temperature,
            max_tokens=req.maxTokens,
            base_url=req.baseUrl,
        )
        return {
            "text": result.text,
            "tokensUsed": result.tokens_used,
            "model": result.model,
            "durationMs": result.duration_ms,
        }
    except Exception as exc:
        logger.error("Local chat failed: %s", exc)
        return {"error": str(exc), "engine": req.engine, "model": req.model}


# ── Artifacts endpoints ─────────────────────────────────────────────────────


@router.get("/artifacts/{workflow_name}/{step_name}")
async def list_step_artifacts(workflow_name: str, step_name: str) -> dict[str, Any]:
    """List all artifacts for a workflow step."""
    from engine.services.step_artifacts import step_artifacts

    artifacts = step_artifacts.list_artifacts(workflow_name, step_name)
    return {"artifacts": artifacts, "total": len(artifacts)}


@router.get("/outputs/{workflow_name}")
async def list_workflow_outputs(workflow_name: str) -> dict[str, Any]:
    """List all run output folders for a workflow."""
    runs = workflow_artifacts.list_runs(workflow_name)
    return {"runs": runs, "total": len(runs)}


@router.get("/outputs/{workflow_name}/{run_id}/{filename:path}")
async def get_workflow_output_file(workflow_name: str, run_id: str, filename: str) -> dict[str, Any]:
    """Read content of a specific file from a workflow run output folder."""
    content = workflow_artifacts.get_file_content(workflow_name, run_id, filename)
    if content is None:
        return {"error": f"File {filename} not found in run {run_id}"}
    return {"content": content, "filename": filename, "runId": run_id}


class SaveConfigRequest(BaseModel):
    workflowName: str
    config: dict[str, Any]
    maxVersions: int = 3


@router.post("/save-config")
async def save_workflow_config(req: SaveConfigRequest) -> dict[str, Any]:
    """Save workflow configuration and plan to disk with versioning."""
    return workflow_artifacts.save_workflow_config(
        workflow_name=req.workflowName,
        config=req.config,
        max_versions=req.maxVersions,
    )
