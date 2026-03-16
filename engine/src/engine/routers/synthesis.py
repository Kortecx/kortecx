"""Data synthesis router — create and manage synthesis jobs."""

from __future__ import annotations

import uuid
from typing import Any

from fastapi import APIRouter, Header
from fastapi.responses import JSONResponse
from pydantic import BaseModel

from engine.services.synthesis import (
    SynthesisConfig, SynthesisSource, OutputFormat, synthesis_service,
)
from engine.services.local_inference import inference_router
from engine.services.hf import hf_service

router = APIRouter()


class CreateSynthesisRequest(BaseModel):
    name: str
    description: str
    source: str                    # "ollama" | "llamacpp" | "huggingface"
    model: str
    baseUrl: str | None = None
    promptTemplate: str = ""
    systemPrompt: str = ""
    targetSamples: int = 100
    outputFormat: str = "jsonl"     # jsonl | csv | alpaca | chatml | sharegpt | delta
    temperature: float = 0.8
    maxTokens: int = 1024
    batchSize: int = 5
    saveToQdrant: bool = False
    qdrantCollection: str = ""
    tags: list[str] = []
    categories: list[str] = []


@router.post("/start")
async def start_synthesis(
    body: CreateSynthesisRequest,
    x_hf_token: str | None = Header(None),
) -> dict[str, Any]:
    """Create and start a data synthesis job."""
    if x_hf_token:
        hf_service.set_token(x_hf_token)

    job_id = f"synth-{uuid.uuid4().hex[:12]}"

    try:
        source = SynthesisSource(body.source)
    except ValueError:
        return JSONResponse(status_code=400, content={"error": f"Invalid source: {body.source}"})

    try:
        fmt = OutputFormat(body.outputFormat)
    except ValueError:
        return JSONResponse(status_code=400, content={"error": f"Invalid format: {body.outputFormat}"})

    config = SynthesisConfig(
        job_id=job_id,
        name=body.name,
        description=body.description,
        source=source,
        model=body.model,
        base_url=body.baseUrl,
        prompt_template=body.promptTemplate,
        system_prompt=body.systemPrompt,
        target_samples=body.targetSamples,
        output_format=fmt,
        temperature=body.temperature,
        max_tokens=body.maxTokens,
        batch_size=body.batchSize,
        save_to_qdrant=body.saveToQdrant,
        qdrant_collection=body.qdrantCollection,
        tags=body.tags,
        categories=body.categories,
    )

    job = synthesis_service.create_job(config)
    await synthesis_service.start_job(job_id)

    return {"jobId": job_id, "status": "running", "message": f"Synthesis started: {body.name}"}


@router.get("/jobs")
async def list_jobs() -> dict[str, Any]:
    """List all synthesis jobs."""
    jobs = synthesis_service.list_jobs()
    active = sum(1 for j in jobs if j["status"] == "running")
    return {"jobs": jobs, "total": len(jobs), "active": active}


@router.get("/jobs/{job_id}")
async def get_job(job_id: str) -> dict[str, Any]:
    """Get status of a specific synthesis job."""
    job = synthesis_service.get_job(job_id)
    if not job:
        return JSONResponse(status_code=404, content={"error": "Job not found"})
    return synthesis_service._job_to_dict(job)


@router.post("/jobs/{job_id}/cancel")
async def cancel_job(job_id: str) -> dict[str, Any]:
    """Cancel a running synthesis job."""
    job = synthesis_service.get_job(job_id)
    if not job:
        return JSONResponse(status_code=404, content={"error": "Job not found"})
    await synthesis_service.cancel_job(job_id)
    return {"jobId": job_id, "status": "cancelled"}


@router.get("/models")
async def list_available_models() -> dict[str, Any]:
    """List locally available models from all sources."""
    result: dict[str, Any] = {"ollama": [], "llamacpp": [], "huggingface": []}

    # Ollama — locally installed models
    try:
        models = await inference_router.list_models("ollama")
        result["ollama"] = [
            {"name": m["name"], "size": m.get("size", 0), "source": "ollama", "local": True}
            for m in models
        ]
    except Exception:
        pass

    # llama.cpp — loaded model
    try:
        models = await inference_router.list_models("llamacpp")
        result["llamacpp"] = [
            {"name": m["name"], "size": m.get("size", 0), "source": "llamacpp", "local": True}
            for m in models
        ]
    except Exception:
        pass

    # Curated HuggingFace models good for data synthesis
    result["huggingface"] = [
        {"name": "google/flan-t5-base", "source": "huggingface", "local": False, "note": "Lightweight, good for structured generation"},
        {"name": "google/flan-t5-large", "source": "huggingface", "local": False, "note": "Better quality, needs more RAM"},
        {"name": "mistralai/Mistral-7B-Instruct-v0.3", "source": "huggingface", "local": False, "note": "Strong instruction following"},
        {"name": "microsoft/Phi-3-mini-4k-instruct", "source": "huggingface", "local": False, "note": "Small and efficient"},
        {"name": "Qwen/Qwen2.5-3B-Instruct", "source": "huggingface", "local": False, "note": "Multilingual, compact"},
    ]

    return result


GEN_TYPE_PIPELINES: dict[str, list[str]] = {
    "text": ["text-generation", "text2text-generation", "summarization", "translation", "fill-mask", "question-answering"],
    "image": ["text-to-image", "image-to-image", "image-classification", "image-segmentation", "unconditional-image-generation"],
    "audio": ["text-to-speech", "text-to-audio", "automatic-speech-recognition", "audio-classification", "audio-to-audio"],
}


@router.get("/models/search")
async def search_models(
    query: str = "",
    source: str = "ollama",
    gen_type: str = "text",
    limit: int = 10,
    x_hf_token: str | None = Header(None),
) -> dict[str, Any]:
    """Search for models on Ollama library or HuggingFace Hub, filtered by generation type."""
    import httpx as _httpx

    if x_hf_token:
        hf_service.set_token(x_hf_token)

    results: list[dict[str, Any]] = []

    if source == "ollama":
        # Search Ollama library via their API
        try:
            async with _httpx.AsyncClient(timeout=10) as client:
                resp = await client.get(
                    "https://ollama.com/api/search",
                    params={"q": query},
                )
                if resp.status_code == 200:
                    data = resp.json()
                    for m in (data if isinstance(data, list) else data.get("models", []))[:limit]:
                        name = m.get("name", "")
                        if not name:
                            continue
                        results.append({
                            "name": name,
                            "description": m.get("description", ""),
                            "source": "ollama",
                            "local": False,
                        })
        except Exception:
            pass

        # Also include matching local models
        try:
            local = await inference_router.list_models("ollama")
            q_lower = query.lower()
            for m in local:
                if q_lower in m["name"].lower():
                    results.insert(0, {
                        "name": m["name"],
                        "description": "Installed locally",
                        "source": "ollama",
                        "local": True,
                    })
        except Exception:
            pass

    elif source == "huggingface":
        # Search HuggingFace Hub — filtered by generation type pipeline tags
        pipelines = GEN_TYPE_PIPELINES.get(gen_type, [])
        try:
            from engine.services.hf import hf_service as _hf
            all_models: list[dict[str, Any]] = []

            if pipelines:
                # Search with each relevant pipeline tag, deduplicate
                seen: set[str] = set()
                for ptag in pipelines[:3]:  # limit to first 3 tags to avoid too many requests
                    models = _hf.search_models(
                        query=query,
                        pipeline_tag=ptag,
                        limit=limit,
                    )
                    for m in models:
                        mid = m.get("id", "")
                        if mid and mid not in seen:
                            seen.add(mid)
                            all_models.append(m)
                    if len(all_models) >= limit:
                        break
            else:
                # No filter — search all
                all_models = _hf.search_models(query=query, pipeline_tag=None, limit=limit)

            for m in all_models[:limit]:
                dl = m.get("downloads", 0)
                tag = m.get("pipeline_tag", "")
                desc_parts = []
                if tag:
                    desc_parts.append(tag)
                if dl:
                    desc_parts.append(f"{dl:,} downloads")
                results.append({
                    "name": m.get("id", ""),
                    "description": " · ".join(desc_parts) if desc_parts else "",
                    "pipeline_tag": tag,
                    "source": "huggingface",
                    "local": False,
                })
        except Exception:
            pass

    elif source == "llamacpp":
        # llama.cpp doesn't have a registry — return local models matching query
        try:
            local = await inference_router.list_models("llamacpp")
            q_lower = query.lower()
            for m in local:
                if not query or q_lower in m["name"].lower():
                    results.append({
                        "name": m["name"],
                        "description": "Loaded in llama.cpp server",
                        "source": "llamacpp",
                        "local": True,
                    })
        except Exception:
            pass

    return {"models": results, "source": source, "query": query}
