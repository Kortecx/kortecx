from __future__ import annotations

from typing import Any

from fastapi import APIRouter
from pydantic import BaseModel

from engine.services.chains import chain_service

router = APIRouter()


class PipelineRequest(BaseModel):
    chain_type: str  # "qa" | "summarize" | "rag"
    inputs: dict[str, Any]
    config: dict[str, Any] | None = None


@router.post("/run")
async def run_pipeline(req: PipelineRequest) -> dict[str, Any]:
    """Execute a LangChain pipeline."""
    import asyncio

    result = await asyncio.to_thread(chain_service.run_chain, req.chain_type, req.inputs, req.config)
    return {"chain_type": req.chain_type, "result": result}


@router.get("/types")
async def list_pipeline_types() -> dict[str, Any]:
    """List available pipeline types."""
    return {
        "types": [
            {
                "name": "qa",
                "description": "Question answering — direct LLM response",
                "required_inputs": ["question"],
                "config": {"model_id": "string (HuggingFace model ID)"},
            },
            {
                "name": "summarize",
                "description": "Text summarization",
                "required_inputs": ["text"],
                "config": {"model_id": "string (HuggingFace model ID)"},
            },
            {
                "name": "rag",
                "description": "Retrieval-augmented generation — searches Qdrant then answers",
                "required_inputs": ["question"],
                "config": {
                    "model_id": "string (HuggingFace model ID)",
                    "embedding_model": "string (sentence-transformers model)",
                    "collection": "string (Qdrant collection name)",
                },
            },
        ]
    }
