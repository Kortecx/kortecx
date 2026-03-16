from __future__ import annotations

from typing import Any

from fastapi import APIRouter
from pydantic import BaseModel

from engine.services.hf import hf_service

router = APIRouter()


class InferenceRequest(BaseModel):
    model_id: str
    inputs: Any
    parameters: dict[str, Any] | None = None


class TextGenRequest(BaseModel):
    model_id: str
    prompt: str
    max_new_tokens: int = 256
    temperature: float = 0.7
    top_p: float = 0.95


@router.post("/run")
async def run_inference(req: InferenceRequest) -> dict[str, Any]:
    """Run inference on a HuggingFace model."""
    result = await hf_service.run_inference(req.model_id, req.inputs, req.parameters)
    return {"model": req.model_id, "result": result}


@router.post("/generate")
async def text_generation(req: TextGenRequest) -> dict[str, Any]:
    """Generate text with a language model."""
    result = hf_service.text_generation(
        model_id=req.model_id,
        prompt=req.prompt,
        max_new_tokens=req.max_new_tokens,
        temperature=req.temperature,
        top_p=req.top_p,
    )
    return {"model": req.model_id, "generated_text": result}
