from __future__ import annotations

import asyncio
from typing import Any
from uuid import uuid4

from fastapi import APIRouter, BackgroundTasks
from pydantic import BaseModel

from engine.core.websocket import ws_manager
from engine.services.training import TrainingConfig, TrainingMethod, training_service

router = APIRouter()

# In-memory job tracker (replace with DB in production)
_jobs: dict[str, dict[str, Any]] = {}


class TrainRequest(BaseModel):
    model_id: str
    dataset_id: str
    method: TrainingMethod = TrainingMethod.SFT
    output_dir: str = "./outputs"
    epochs: int = 3
    batch_size: int = 4
    learning_rate: float = 2e-5
    max_seq_length: int = 2048
    lora_r: int = 16
    lora_alpha: int = 32
    lora_dropout: float = 0.05
    use_unsloth: bool = True


@router.post("/start")
async def start_training(req: TrainRequest, background_tasks: BackgroundTasks) -> dict[str, Any]:
    """Start a fine-tuning job (runs in background)."""
    job_id = str(uuid4())
    config = TrainingConfig(
        model_id=req.model_id,
        dataset_id=req.dataset_id,
        method=req.method,
        output_dir=f"{req.output_dir}/{job_id}",
        epochs=req.epochs,
        batch_size=req.batch_size,
        learning_rate=req.learning_rate,
        max_seq_length=req.max_seq_length,
        lora_r=req.lora_r,
        lora_alpha=req.lora_alpha,
        lora_dropout=req.lora_dropout,
        use_unsloth=req.use_unsloth,
    )

    _jobs[job_id] = {"id": job_id, "status": "queued", "config": req.model_dump()}
    background_tasks.add_task(_run_training, job_id, config)

    return {"job_id": job_id, "status": "queued"}


async def _run_training(job_id: str, config: TrainingConfig) -> None:
    _jobs[job_id]["status"] = "running"
    await ws_manager.broadcast("training", "training.started", {"job_id": job_id})

    try:
        if config.method == TrainingMethod.SFT:
            result = await asyncio.to_thread(training_service.sft_train, config)
        elif config.method == TrainingMethod.DPO:
            result = await asyncio.to_thread(training_service.dpo_train, config)
        else:
            result = await asyncio.to_thread(training_service.sft_train, config)

        _jobs[job_id].update({"status": "completed", "result": result})
        await ws_manager.broadcast("training", "training.completed", {"job_id": job_id, "result": result})
    except Exception as e:
        _jobs[job_id].update({"status": "failed", "error": str(e)})
        await ws_manager.broadcast("training", "training.failed", {"job_id": job_id, "error": str(e)})


@router.get("/jobs")
async def list_jobs() -> dict[str, Any]:
    return {"jobs": list(_jobs.values()), "count": len(_jobs)}


@router.get("/jobs/{job_id}")
async def get_job(job_id: str) -> dict[str, Any]:
    if job_id not in _jobs:
        from fastapi import HTTPException
        raise HTTPException(404, "Job not found")
    return _jobs[job_id]
