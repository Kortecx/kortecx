"""Orchestrator API — workflow execution, file uploads, run status."""

from __future__ import annotations

import logging
import os
import uuid
from pathlib import Path
from typing import Any

from fastapi import APIRouter, BackgroundTasks, UploadFile, File
from pydantic import BaseModel

from engine.config import settings
from engine.services.local_inference import inference_router, OllamaService, model_pool
from engine.services.orchestrator import (
    AgentOrchestrator, StepConfig, StepIntegration, WorkflowRequest, orchestrator,
)

logger = logging.getLogger("engine.routers.orchestrator")

router = APIRouter()


# ── Request / Response models ────────────────────────────────────────────────

class LocalModelConfigModel(BaseModel):
    engine: str = "ollama"     # ollama | llamacpp
    model: str = "llama3.1:8b"
    baseUrl: str | None = None


class StepIntegrationModel(BaseModel):
    id: str
    type: str                  # integration | plugin
    referenceId: str
    name: str
    icon: str = ""
    color: str = ""
    config: dict[str, str] = {}


class StepConfigModel(BaseModel):
    stepId: str
    expertId: str | None = None
    taskDescription: str
    systemInstructions: str = ""
    voiceCommand: str = ""
    fileLocations: list[str] = []
    stepFileNames: list[str] = []
    stepImageNames: list[str] = []
    modelSource: str = "local"          # local | provider
    localModel: LocalModelConfigModel | None = None
    temperature: float = 0.7
    maxTokens: int = 4096
    connectionType: str = "sequential"  # sequential | parallel
    integrations: list[StepIntegrationModel] = []


class ExecuteRequest(BaseModel):
    workflowId: str | None = None
    name: str
    goalFileUrl: str
    inputFileUrls: list[str] = []
    steps: list[StepConfigModel]


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
        steps=[
            StepConfig(
                step_id=s.stepId,
                expert_id=s.expertId,
                task_description=s.taskDescription,
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
                        id=si.id, type=si.type, reference_id=si.referenceId,
                        name=si.name, icon=si.icon, color=si.color,
                        config=si.config,
                    )
                    for si in s.integrations
                ],
            )
            for s in req.steps
        ],
    )

    # Run orchestration in background so we return immediately
    bg.add_task(orchestrator.execute_workflow, request)

    return ExecuteResponse(
        runId=request.workflow_id,
        status="started",
        message=f"Workflow '{req.name}' execution started — {len(req.steps)} agent(s) will be spawned",
    )


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

        uploaded.append({
            "id": file_id,
            "filename": f.filename or filename,
            "storedAs": filename,
            "url": f"/uploads/{filename}",
            "size": len(content),
        })
        logger.info("Uploaded: %s → %s (%d bytes)", f.filename, filename, len(content))

    return {"files": uploaded, "count": len(uploaded)}


# ── Monitoring endpoints ──────────────────────────────────────────────────────

@router.get("/status")
async def orchestrator_status() -> dict[str, Any]:
    """Get orchestrator runtime status including system resources."""
    from engine.services.system_stats import get_system_stats, get_process_stats
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
