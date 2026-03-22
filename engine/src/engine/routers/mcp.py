from __future__ import annotations

import json
import time
from typing import Any

try:
    import psutil
except ImportError:
    psutil = None  # type: ignore[assignment]

from fastapi import APIRouter
from fastapi.responses import StreamingResponse
from pydantic import BaseModel

from engine.services.local_inference import inference_router as local_inference
from engine.services.mcp import mcp_service

router = APIRouter()


class GenerateMcpRequest(BaseModel):
    prompt: str
    description: str = ""
    language: str = "python"  # python | typescript | javascript
    model: str = "llama3.1:8b"
    source: str = "ollama"  # ollama | llamacpp | provider
    provider_id: str = ""  # anthropic | openai | google — used when source=provider
    system_prompt: str = ""
    prompt_type: str = "mcp"  # mcp | data_synthesis | training | finetuning | general


class CacheScriptRequest(BaseModel):
    name: str
    description: str
    language: str = "python"
    code: str
    filename: str | None = None


class UpdateScriptRequest(BaseModel):
    code: str | None = None
    description: str | None = None
    is_public: bool | None = None


class PersistRequest(BaseModel):
    script_id: str


# ── Discovery ────────────────────────────────────────────────────────────────


@router.get("/servers")
async def list_servers() -> dict[str, Any]:
    """List all MCP servers — prebuilt, persisted, and session-cached."""
    prebuilt = mcp_service.list_prebuilt()
    persisted = mcp_service.list_persisted()
    cached = mcp_service.list_cached()
    return {
        "prebuilt": prebuilt,
        "persisted": persisted,
        "cached": cached,
        "total": len(prebuilt) + len(persisted) + len(cached),
        "max_versions": mcp_service.max_versions,
    }


@router.get("/servers/{script_id}")
async def get_server(script_id: str) -> dict[str, Any]:
    """Get details of a single MCP server (cached, prebuilt, or persisted)."""
    cached = mcp_service.get_cached(script_id)
    if cached:
        return {"server": cached}
    # Check prebuilt/persisted
    for s in mcp_service.list_prebuilt() + mcp_service.list_persisted():
        if s["id"] == script_id:
            return {"server": s}
    return {"error": "Server not found"}


# ── AI Generation ────────────────────────────────────────────────────────────


@router.post("/generate")
async def generate_mcp_script(req: GenerateMcpRequest) -> dict[str, Any]:
    """Generate an MCP server script using a local LLM."""
    # Use user-provided system prompt or fall back to default
    sys_prompt = (
        req.system_prompt.strip()
        if req.system_prompt.strip()
        else (
            "You are an expert MCP (Model Context Protocol) server developer. "
            "Generate a complete, working MCP server script. "
            "The script must be self-contained and runnable. "
            "Include proper imports, tool definitions, and a main entry point. "
            "Only output the code — no explanations, no markdown fences."
        )
    )

    full_prompt = f"{sys_prompt}\n\nUser request: {req.prompt}"

    try:
        cpu_before = psutil.cpu_percent(interval=None) if psutil else 0.0
        t0 = time.monotonic()

        result = await local_inference.generate(
            engine=req.source,
            model=req.model,
            prompt=full_prompt,
            max_tokens=4096,
            temperature=0.3,
        )

        generation_time_ms = int((time.monotonic() - t0) * 1000)
        cpu_after = psutil.cpu_percent(interval=None) if psutil else 0.0
        avg_cpu = round((cpu_before + cpu_after) / 2, 1)

        code = result.text
        # Strip markdown fences if present
        if code.startswith("```"):
            lines = code.split("\n")
            lines = lines[1:]  # remove opening fence
            if lines and lines[-1].strip() == "```":
                lines = lines[:-1]
            code = "\n".join(lines)

        # Extract a name from the prompt
        name_words = req.prompt.split()[:5]
        name = " ".join(name_words).title()

        description = req.description.strip() if req.description.strip() else req.prompt

        # Cache it
        server = mcp_service.cache_script(
            name=name,
            description=description,
            language=req.language,
            code=code,
            prompt=req.prompt,
            generation_time_ms=generation_time_ms,
            cpu_percent=avg_cpu,
            prompt_type=req.prompt_type,
        )
        return {"server": server, "generated": True, "generation_time_ms": generation_time_ms, "cpu_percent": avg_cpu}

    except Exception as exc:
        return {"error": str(exc), "generated": False}


@router.post("/generate/stream")
async def generate_mcp_script_stream(req: GenerateMcpRequest) -> StreamingResponse:
    """Stream-generate an MCP server script — returns SSE with token chunks."""
    sys_prompt = (
        req.system_prompt.strip()
        if req.system_prompt.strip()
        else (
            "You are an expert MCP (Model Context Protocol) server developer. "
            "Generate a complete, working MCP server script. "
            "The script must be self-contained and runnable. "
            "Include proper imports, tool definitions, and a main entry point. "
            "Only output the code — no explanations, no markdown fences."
        )
    )
    full_prompt = f"{sys_prompt}\n\nUser request: {req.prompt}"

    async def _stream_provider():
        """Stream from a cloud provider (Anthropic/OpenAI/Google) using stored API key."""
        import httpx

        # Fetch the API key from the frontend DB via internal API
        # The engine calls the frontend's provider key endpoint
        frontend_url = "http://localhost:3000"
        provider_id = req.provider_id

        # Provider-specific streaming endpoints
        provider_configs: dict[str, dict[str, Any]] = {
            "anthropic": {
                "url": "https://api.anthropic.com/v1/messages",
                "headers_fn": lambda key: {"x-api-key": key, "anthropic-version": "2023-06-01", "content-type": "application/json"},
                "body_fn": lambda: {"model": req.model, "max_tokens": 4096, "stream": True, "messages": [{"role": "user", "content": full_prompt}]},
                "token_path": lambda chunk: chunk.get("delta", {}).get("text", ""),
                "stop_check": lambda chunk: chunk.get("type") == "message_stop",
            },
            "openai": {
                "url": "https://api.openai.com/v1/chat/completions",
                "headers_fn": lambda key: {"Authorization": f"Bearer {key}", "content-type": "application/json"},
                "body_fn": lambda: {
                    "model": req.model,
                    "max_tokens": 4096,
                    "stream": True,
                    "messages": [{"role": "system", "content": sys_prompt}, {"role": "user", "content": req.prompt}],
                },
                "token_path": lambda chunk: chunk.get("choices", [{}])[0].get("delta", {}).get("content") or "",
                "stop_check": lambda chunk: chunk.get("choices", [{}])[0].get("finish_reason") is not None,
            },
            "google": {
                "url": f"https://generativelanguage.googleapis.com/v1beta/models/{req.model}:streamGenerateContent",
                "headers_fn": lambda key: {"content-type": "application/json"},
                "body_fn": lambda: {"contents": [{"parts": [{"text": full_prompt}]}], "generationConfig": {"maxOutputTokens": 4096, "temperature": 0.3}},
                "token_path": lambda chunk: "".join(p.get("text", "") for c in chunk.get("candidates", []) for p in c.get("content", {}).get("parts", [])),
                "stop_check": lambda _: False,
                "url_fn": lambda key: f"https://generativelanguage.googleapis.com/v1beta/models/{req.model}:streamGenerateContent?alt=sse&key={key}",
            },
        }

        cfg = provider_configs.get(provider_id)
        if not cfg:
            yield f"data: {json.dumps({'type': 'error', 'error': f'Unsupported provider: {provider_id}'})}\n\n"
            return

        # Get API key — call frontend internal API
        try:
            async with httpx.AsyncClient(timeout=10) as c:
                await c.get(f"{frontend_url}/api/providers")  # check connectivity
                # We can't get the raw key from the providers list endpoint
                # Instead, we need to read it from the DB directly
                # For now, use the encrypted key endpoint pattern
            # Fallback: use environment variable
            import os

            env_map = {
                "anthropic": "ANTHROPIC_API_KEY",
                "openai": "OPENAI_API_KEY",
                "google": "GOOGLE_API_KEY",
                "groq": "GROQ_API_KEY",
                "mistral": "MISTRAL_API_KEY",
                "deepseek": "DEEPSEEK_API_KEY",
                "xai": "XAI_API_KEY",
            }
            api_key = os.environ.get(env_map.get(provider_id, ""), "")
            if not api_key:
                env_name = env_map.get(provider_id, "UNKNOWN")
                err_msg = f"No API key found for {provider_id}. Set {env_name} env var."
                yield f"data: {json.dumps({'type': 'error', 'error': err_msg})}\n\n"
                return
        except Exception as e:
            yield f"data: {json.dumps({'type': 'error', 'error': str(e)})}\n\n"
            return

        url = cfg.get("url_fn", lambda k: cfg["url"])(api_key)
        headers = cfg["headers_fn"](api_key)
        body = cfg["body_fn"]()

        try:
            async with httpx.AsyncClient(timeout=300) as client:
                async with client.stream("POST", url, json=body, headers=headers) as resp:
                    resp.raise_for_status()
                    async for line in resp.aiter_lines():
                        if not line.strip():
                            continue
                        text = line
                        if text.startswith("data: "):
                            text = text[6:]
                        if text.strip() == "[DONE]":
                            return
                        try:
                            chunk = json.loads(text)
                            token = cfg["token_path"](chunk)
                            if token:
                                yield token
                            if cfg["stop_check"](chunk):
                                return
                        except json.JSONDecodeError:
                            continue
        except httpx.HTTPStatusError as e:
            yield f"__ERROR__:{e.response.status_code} {e.response.text[:200]}"
        except Exception as e:
            yield f"__ERROR__:{str(e)}"

    async def event_stream():
        cpu_before = psutil.cpu_percent(interval=None) if psutil else 0.0
        t0 = time.monotonic()
        collected: list[str] = []
        error_msg = ""

        try:
            if req.source == "provider" and req.provider_id:
                token_gen = _stream_provider()
            else:
                token_gen = local_inference.generate_stream(
                    engine=req.source,
                    model=req.model,
                    prompt=full_prompt,
                    max_tokens=4096,
                    temperature=0.3,
                )

            async for token in token_gen:
                if token.startswith("__ERROR__:"):
                    error_msg = token[10:]
                    yield f"data: {json.dumps({'type': 'error', 'error': error_msg})}\n\n"
                    break
                collected.append(token)
                yield f"data: {json.dumps({'type': 'token', 'token': token})}\n\n"
        except Exception as exc:
            error_msg = str(exc)
            yield f"data: {json.dumps({'type': 'error', 'error': error_msg})}\n\n"

        generation_time_ms = int((time.monotonic() - t0) * 1000)
        cpu_after = psutil.cpu_percent(interval=None) if psutil else 0.0
        avg_cpu = round((cpu_before + cpu_after) / 2, 1)

        if not error_msg:
            code = "".join(collected)
            # Strip markdown fences
            if code.startswith("```"):
                lines = code.split("\n")
                lines = lines[1:]
                if lines and lines[-1].strip() == "```":
                    lines = lines[:-1]
                code = "\n".join(lines)

            name_words = req.prompt.split()[:5]
            name = " ".join(name_words).title()
            description = req.description.strip() if req.description.strip() else req.prompt

            server = mcp_service.cache_script(
                name=name,
                description=description,
                language=req.language,
                code=code,
                prompt=req.prompt,
                generation_time_ms=generation_time_ms,
                cpu_percent=avg_cpu,
                prompt_type=req.prompt_type,
            )

            yield f"data: {json.dumps({'type': 'done', 'server': server, 'generation_time_ms': generation_time_ms, 'cpu_percent': avg_cpu})}\n\n"

    return StreamingResponse(
        event_stream(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "X-Accel-Buffering": "no",
        },
    )


# ── Caching ──────────────────────────────────────────────────────────────────


@router.post("/cache")
async def cache_script(req: CacheScriptRequest) -> dict[str, Any]:
    """Cache a script in session memory."""
    server = mcp_service.cache_script(
        name=req.name,
        description=req.description,
        language=req.language,
        code=req.code,
        filename=req.filename,
    )
    return {"server": server}


@router.put("/cache/{script_id}")
async def update_cached_script(script_id: str, req: UpdateScriptRequest) -> dict[str, Any]:
    """Update fields of a cached script."""
    server = mcp_service.update_cached(
        script_id,
        code=req.code,
        description=req.description,
        is_public=req.is_public,
    )
    if not server:
        return {"error": "Script not found in cache"}
    return {"server": server}


@router.delete("/cache/{script_id}")
async def delete_cached(script_id: str) -> dict[str, Any]:
    """Remove a cached script."""
    ok = mcp_service.delete_cached(script_id)
    return {"deleted": ok}


# ── Testing ──────────────────────────────────────────────────────────────────


@router.post("/test/{script_id}")
async def test_script(script_id: str) -> dict[str, Any]:
    """Execute a cached script to test it."""
    return await mcp_service.test_script(script_id)


# ── Persistence ──────────────────────────────────────────────────────────────


@router.post("/persist/{script_id}")
async def persist_script(script_id: str) -> dict[str, Any]:
    """Persist a cached script to engine/mcp_scripts/."""
    server = mcp_service.persist_script(script_id)
    if not server:
        return {"error": "Script not found in cache"}
    return {"server": server, "persisted": True}


@router.delete("/persisted/{script_id}")
async def delete_persisted(script_id: str) -> dict[str, Any]:
    """Delete a persisted MCP script file."""
    ok = mcp_service.delete_persisted(script_id)
    return {"deleted": ok}


# ── Versioning ──────────────────────────────────────────────────────────────


class SetMaxVersionsRequest(BaseModel):
    max_versions: int


@router.get("/versions/{script_id}")
async def list_versions(script_id: str) -> dict[str, Any]:
    """List version history for a persisted script."""
    return {"versions": mcp_service.list_versions(script_id), "max_versions": mcp_service.max_versions}


@router.put("/config/max-versions")
async def set_max_versions(req: SetMaxVersionsRequest) -> dict[str, Any]:
    """Set the maximum number of versions to keep per script."""
    n = mcp_service.set_max_versions(req.max_versions)
    return {"max_versions": n}


@router.get("/config")
async def get_config() -> dict[str, Any]:
    """Get MCP service configuration."""
    return {"max_versions": mcp_service.max_versions}
