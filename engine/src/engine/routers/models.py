from __future__ import annotations

import asyncio
import logging
import time
from pathlib import Path
from typing import Any

from fastapi import APIRouter, Query

from engine.config import settings
from engine.services.hf import hf_service
from engine.services.local_inference import inference_router
from engine.services.mlflow_tracker import mlflow_tracker

logger = logging.getLogger("engine.models")

router = APIRouter()


@router.get("/search")
async def search_models(
    query: str = "",
    pipeline_tag: str | None = None,
    library: str | None = None,
    sort: str = "downloads",
    limit: int = Query(20, ge=1, le=100),
) -> dict[str, Any]:
    """Search HuggingFace Hub models."""
    models = hf_service.search_models(
        query=query,
        pipeline_tag=pipeline_tag,
        library=library,
        sort=sort,
        limit=limit,
    )
    return {"models": models, "count": len(models)}


def _extract_text(path: Path, max_chars: int = 50_000) -> str:
    """Extract text content from a file. Supports text and PDF."""
    suffix = path.suffix.lower()
    if suffix == ".pdf":
        try:
            import fitz  # PyMuPDF

            doc = fitz.open(str(path))
            text = "\n".join(page.get_text() for page in doc)
            doc.close()
            return text[:max_chars]
        except ImportError:
            return "[PDF extraction unavailable — install PyMuPDF: pip install pymupdf]"
        except Exception as exc:
            return f"[PDF extraction error: {exc}]"
    try:
        return path.read_text(encoding="utf-8", errors="replace")[:max_chars]
    except Exception as exc:
        return f"[File read error: {exc}]"


@router.post("/compare")
async def compare_models(req: dict[str, Any]) -> dict[str, Any]:
    """Run two models concurrently and compare their outputs."""
    prompt = req.get("prompt", "")
    system_prompt = req.get("system_prompt", "")
    model_a = req.get("model_a", "")
    engine_a = req.get("engine_a", "ollama")
    model_b = req.get("model_b", "")
    engine_b = req.get("engine_b", "ollama")
    temperature = float(req.get("temperature", 0.7))
    max_tokens = int(req.get("max_tokens", 4096))
    document_urls: list[str] = req.get("document_urls", [])

    # Extract document text and prepend to prompt
    doc_names: list[str] = []
    doc_context = ""
    if document_urls:
        upload_dir = Path(settings.upload_dir)
        for url in document_urls:
            basename = url.rsplit("/", 1)[-1]
            file_path = upload_dir / basename
            if file_path.exists():
                doc_names.append(basename)
                content = _extract_text(file_path)
                doc_context += f"\n---\n[Document: {basename}]\n{content}\n---\n"

    effective_prompt = (doc_context + "\n" + prompt) if doc_context else prompt

    messages = []
    if system_prompt:
        messages.append({"role": "system", "content": system_prompt})
    messages.append({"role": "user", "content": effective_prompt})

    async def run_model(engine: str, model: str) -> dict[str, Any]:
        start = time.time()
        try:
            result = await inference_router.chat(
                engine,
                model,
                messages,
                temperature=temperature,
                max_tokens=max_tokens,
            )
            duration_ms = (time.time() - start) * 1000
            tokens = result.tokens_used
            tokens_per_sec = tokens / (duration_ms / 1000) if duration_ms > 0 else 0
            return {
                "model": model,
                "engine": engine,
                "response": result.text,
                "tokens": tokens,
                "duration_ms": round(duration_ms),
                "tokens_per_sec": round(tokens_per_sec, 1),
                "error": None,
            }
        except Exception as exc:
            duration_ms = (time.time() - start) * 1000
            logger.warning("Compare model %s/%s failed: %s", engine, model, exc)
            return {
                "model": model,
                "engine": engine,
                "response": "",
                "tokens": 0,
                "duration_ms": round(duration_ms),
                "tokens_per_sec": 0,
                "error": str(exc),
            }

    result_a, result_b = await asyncio.gather(
        run_model(engine_a, model_a),
        run_model(engine_b, model_b),
    )

    # Log to MLflow if enabled
    mlflow_run_id: str | None = None
    if not result_a.get("error") and not result_b.get("error"):
        mlflow_run_id = mlflow_tracker.log_comparison(
            model_a=model_a,
            model_b=model_b,
            metrics_a={
                "tokens": result_a["tokens"],
                "duration_ms": result_a["duration_ms"],
                "tokens_per_sec": result_a["tokens_per_sec"],
            },
            metrics_b={
                "tokens": result_b["tokens"],
                "duration_ms": result_b["duration_ms"],
                "tokens_per_sec": result_b["tokens_per_sec"],
            },
            prompt=prompt,
            temperature=temperature,
            document_count=len(doc_names),
        )

    return {
        "model_a": result_a,
        "model_b": result_b,
        "temperature": temperature,
        "prompt": prompt,
        "mlflow_run_id": mlflow_run_id,
        "document_count": len(doc_names),
        "document_names": doc_names,
    }


@router.get("/{model_id:path}")
async def get_model_info(model_id: str) -> dict[str, Any]:
    """Get detailed info for a specific model."""
    return hf_service.get_model_info(model_id)
