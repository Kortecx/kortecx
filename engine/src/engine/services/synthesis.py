"""Data synthesis service — generates datasets using local or HF models."""

from __future__ import annotations

import asyncio
import csv
import json
import logging
import time
from dataclasses import dataclass, field
from datetime import UTC, datetime
from enum import StrEnum
from pathlib import Path
from typing import Any

from engine.config import settings
from engine.services.local_inference import inference_router, model_pool

logger = logging.getLogger("engine.synthesis")


class SynthesisSource(StrEnum):
    OLLAMA = "ollama"
    LLAMACPP = "llamacpp"
    HUGGINGFACE = "huggingface"


class SynthesisStatus(StrEnum):
    QUEUED = "queued"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


class OutputFormat(StrEnum):
    JSONL = "jsonl"
    CSV = "csv"
    ALPACA = "alpaca"
    CHATML = "chatml"
    SHAREGPT = "sharegpt"
    DELTA = "delta"


@dataclass
class SynthesisConfig:
    job_id: str
    name: str
    description: str
    source: SynthesisSource
    model: str
    base_url: str | None = None
    prompt_template: str = ""
    system_prompt: str = ""
    target_samples: int = 100
    output_format: OutputFormat = OutputFormat.JSONL
    temperature: float = 0.8
    max_tokens: int = 1024
    batch_size: int = 5  # concurrent generation requests per batch
    save_to_qdrant: bool = False
    qdrant_collection: str = ""
    schema: list[dict] | None = None
    tags: list[str] = field(default_factory=list)
    categories: list[str] = field(default_factory=list)


@dataclass
class SynthesisJob:
    config: SynthesisConfig
    status: SynthesisStatus = SynthesisStatus.QUEUED
    current_samples: int = 0
    samples: list[dict[str, Any]] = field(default_factory=list)
    started_at: str | None = None
    completed_at: str | None = None
    error: str | None = None
    output_path: str | None = None
    tokens_used: int = 0
    duration_ms: float = 0
    cost_usd: float = 0


# Format-specific prompt templates
FORMAT_TEMPLATES = {
    OutputFormat.ALPACA: (
        "Generate a training example in Alpaca format with these fields:\n"
        "- instruction: A clear task instruction\n"
        "- input: Optional context or input data\n"
        "- output: The expected response\n\n"
        "Return ONLY valid JSON with keys: instruction, input, output\n\n"
        "Topic/Domain: {description}\n"
        "Example #{sample_num}:"
    ),
    OutputFormat.CHATML: (
        "Generate a conversation training example with these fields:\n"
        "- system: System message setting the context\n"
        "- user: User message or question\n"
        "- assistant: Assistant response\n\n"
        "Return ONLY valid JSON with keys: system, user, assistant\n\n"
        "Topic/Domain: {description}\n"
        "Example #{sample_num}:"
    ),
    OutputFormat.SHAREGPT: (
        "Generate a multi-turn conversation example with this structure:\n"
        "- conversations: Array of {{from, value}} objects where from is 'human' or 'gpt'\n\n"
        "Return ONLY valid JSON with key: conversations (array of 2-4 turns)\n\n"
        "Topic/Domain: {description}\n"
        "Example #{sample_num}:"
    ),
    OutputFormat.JSONL: (
        "Generate a structured data sample as valid JSON.\n"
        "The data should be relevant to: {description}\n\n"
        "Return ONLY a single valid JSON object.\n"
        "Example #{sample_num}:"
    ),
    OutputFormat.CSV: (
        "Generate a structured data record as valid JSON with consistent field names.\n"
        "The data should be relevant to: {description}\n\n"
        "Return ONLY a single valid JSON object with string/number values (no nested objects).\n"
        "Example #{sample_num}:"
    ),
    OutputFormat.DELTA: (
        "Generate a structured data record as valid JSON with consistent field names.\n"
        "The data should be relevant to: {description}\n"
        "Include a timestamp field for temporal tracking.\n\n"
        "Return ONLY a single valid JSON object.\n"
        "Example #{sample_num}:"
    ),
}


class SynthesisService:
    """Manages parallel data synthesis jobs."""

    def __init__(self) -> None:
        self._jobs: dict[str, SynthesisJob] = {}
        self._tasks: dict[str, asyncio.Task[None]] = {}
        self._semaphore = asyncio.Semaphore(settings.max_concurrent_agents)

    def create_job(self, config: SynthesisConfig) -> SynthesisJob:
        job = SynthesisJob(config=config)
        self._jobs[config.job_id] = job
        return job

    async def start_job(self, job_id: str) -> None:
        """Start a synthesis job in the background."""
        job = self._jobs.get(job_id)
        if not job:
            raise ValueError(f"Job {job_id} not found")
        if job.status == SynthesisStatus.RUNNING:
            raise ValueError(f"Job {job_id} already running")

        task = asyncio.create_task(self._run_job(job))
        self._tasks[job_id] = task

    async def cancel_job(self, job_id: str) -> None:
        task = self._tasks.get(job_id)
        if task and not task.done():
            task.cancel()
        job = self._jobs.get(job_id)
        if job:
            job.status = SynthesisStatus.CANCELLED

    def get_job(self, job_id: str) -> SynthesisJob | None:
        return self._jobs.get(job_id)

    def list_jobs(self) -> list[dict[str, Any]]:
        return [self._job_to_dict(j) for j in self._jobs.values()]

    def _job_to_dict(self, job: SynthesisJob) -> dict[str, Any]:
        return {
            "jobId": job.config.job_id,
            "name": job.config.name,
            "description": job.config.description,
            "source": job.config.source.value,
            "model": job.config.model,
            "status": job.status.value,
            "targetSamples": job.config.target_samples,
            "currentSamples": job.current_samples,
            "outputFormat": job.config.output_format.value,
            "temperature": job.config.temperature,
            "startedAt": job.started_at,
            "completedAt": job.completed_at,
            "error": job.error,
            "outputPath": job.output_path,
            "tokensUsed": job.tokens_used,
            "durationMs": job.duration_ms,
            "tags": job.config.tags,
            "progress": round(job.current_samples / max(job.config.target_samples, 1) * 100),
        }

    async def _run_job(self, job: SynthesisJob) -> None:
        """Execute a synthesis job — generate samples in batches."""
        cfg = job.config
        job.status = SynthesisStatus.RUNNING
        job.started_at = datetime.now(UTC).isoformat()
        start_time = time.monotonic()

        # Ensure output directory exists
        output_dir = Path(settings.upload_dir) / "synthesis"
        output_dir.mkdir(parents=True, exist_ok=True)

        try:
            # Generate in batches
            remaining = cfg.target_samples
            while remaining > 0 and job.status == SynthesisStatus.RUNNING:
                batch_size = min(cfg.batch_size, remaining)
                batch_start = cfg.target_samples - remaining

                # Run batch concurrently
                tasks = [self._generate_sample(cfg, batch_start + i + 1) for i in range(batch_size)]
                results = await asyncio.gather(*tasks, return_exceptions=True)

                for r in results:
                    if isinstance(r, Exception):
                        logger.warning("Sample generation failed: %s", r)
                        continue
                    if r is not None:
                        sample, tokens = r
                        job.samples.append(sample)
                        job.current_samples += 1
                        job.tokens_used += tokens

                remaining = cfg.target_samples - job.current_samples

                # Brief yield to allow cancellation
                await asyncio.sleep(0.01)

            # Save output files
            elapsed = time.monotonic() - start_time
            job.duration_ms = elapsed * 1000

            if job.samples:
                output_path = await self._save_output(job, output_dir)
                job.output_path = str(output_path)

                # Optionally store in Qdrant
                if cfg.save_to_qdrant and cfg.qdrant_collection:
                    await self._store_in_qdrant(job)

            job.status = SynthesisStatus.COMPLETED
            job.completed_at = datetime.now(UTC).isoformat()
            logger.info(
                "Synthesis job %s completed: %d/%d samples in %.1fs",
                cfg.job_id,
                job.current_samples,
                cfg.target_samples,
                elapsed,
            )

        except asyncio.CancelledError:
            job.status = SynthesisStatus.CANCELLED
            logger.info("Synthesis job %s cancelled", cfg.job_id)
        except Exception as exc:
            job.status = SynthesisStatus.FAILED
            job.error = str(exc)
            job.completed_at = datetime.now(UTC).isoformat()
            logger.exception("Synthesis job %s failed", cfg.job_id)

    async def _generate_sample(self, cfg: SynthesisConfig, sample_num: int) -> tuple[dict[str, Any], int] | None:
        """Generate a single data sample using the configured model."""
        # Build prompt — when schema is defined, override the generic template
        if cfg.schema:
            field_names = [c["name"] for c in cfg.schema]
            cols_desc = "\n".join(
                f"  - \"{c['name']}\" ({c.get('type','string')}): {c.get('description','')}"
                + (" [REQUIRED]" if c.get("required") else "")
                for c in cfg.schema
            )
            example_obj = ", ".join(f'"{n}": <{c.get("type","string")}>' for n, c in zip(field_names, cfg.schema))
            user_prompt = (
                f"Generate a single JSON object for: {cfg.description}\n\n"
                f"STRICT SCHEMA — the JSON MUST have exactly these keys, no more, no less:\n{cols_desc}\n\n"
                f"Example structure: {{{example_obj}}}\n\n"
                f"Return ONLY a valid JSON object with exactly {len(field_names)} keys: {', '.join(field_names)}\n"
                f"Sample #{sample_num}:"
            )

            system = cfg.system_prompt or (
                "You are a precise data generation assistant. "
                "You MUST follow the provided schema exactly. "
                "Return ONLY a single valid JSON object with the exact field names specified. "
                "No extra fields. No missing fields. No markdown. No explanation."
            )
        else:
            template = FORMAT_TEMPLATES.get(cfg.output_format, FORMAT_TEMPLATES[OutputFormat.JSONL])
            user_prompt = template.format(
                description=cfg.description,
                sample_num=sample_num,
            )

            system = cfg.system_prompt or (
                "You are a precise data generation assistant. "
                "Generate high-quality, diverse, realistic training data samples. "
                "Always return ONLY valid JSON — no markdown, no explanation, no code fences."
            )

        if cfg.prompt_template:
            user_prompt = cfg.prompt_template + "\n\n" + user_prompt

        async with self._semaphore:
            if cfg.source in (SynthesisSource.OLLAMA, SynthesisSource.LLAMACPP):
                engine = cfg.source.value
                await model_pool.acquire(cfg.model)
                try:
                    result = await inference_router.chat(
                        engine=engine,
                        model=cfg.model,
                        messages=[
                            {"role": "system", "content": system},
                            {"role": "user", "content": user_prompt},
                        ],
                        temperature=cfg.temperature,
                        max_tokens=cfg.max_tokens,
                        base_url=cfg.base_url,
                    )
                finally:
                    await model_pool.release(cfg.model)

                text = result.text.strip()
                tokens = result.tokens_used

            elif cfg.source == SynthesisSource.HUGGINGFACE:
                # Use HuggingFace Transformers pipeline
                text, tokens = await asyncio.to_thread(
                    self._hf_generate,
                    cfg.model,
                    system,
                    user_prompt,
                    cfg.temperature,
                    cfg.max_tokens,
                )
            else:
                return None

        # Parse JSON from response
        sample = self._parse_json(text)
        if sample is None:
            logger.debug("Failed to parse sample #%d, retrying once", sample_num)
            return None

        # Enforce schema: keep only declared fields, fill missing with defaults
        if cfg.schema:
            schema_names = {c["name"] for c in cfg.schema}
            filtered = {}
            for c in cfg.schema:
                col_name = c["name"]
                if col_name in sample:
                    filtered[col_name] = sample[col_name]
                else:
                    # Fill missing with type-appropriate default
                    t = c.get("type", "string")
                    filtered[col_name] = (
                        0 if t in ("integer", "float", "number") else
                        False if t == "boolean" else
                        [] if t in ("array", "json") else
                        ""
                    )
            sample = filtered

        # Add metadata
        sample["_meta"] = {
            "sample_id": sample_num,
            "model": cfg.model,
            "source": cfg.source.value,
            "generated_at": datetime.now(UTC).isoformat(),
        }

        return sample, tokens

    def _hf_generate(
        self,
        model_id: str,
        system: str,
        prompt: str,
        temperature: float,
        max_tokens: int,
    ) -> tuple[str, int]:
        """Run generation using HuggingFace Transformers (blocking — run in thread)."""
        try:
            import torch
            from transformers import AutoModelForCausalLM, AutoTokenizer, pipeline

            tokenizer = AutoTokenizer.from_pretrained(model_id)
            model = AutoModelForCausalLM.from_pretrained(
                model_id,
                torch_dtype=torch.float16 if torch.cuda.is_available() else torch.float32,
                device_map="auto" if torch.cuda.is_available() else None,
                low_cpu_mem_usage=True,
            )

            pipe = pipeline(
                "text-generation",
                model=model,
                tokenizer=tokenizer,
                max_new_tokens=max_tokens,
                temperature=temperature,
                do_sample=temperature > 0,
            )

            full_prompt = f"{system}\n\n{prompt}"
            outputs = pipe(full_prompt, return_full_text=False)
            text = outputs[0]["generated_text"].strip()
            tokens = len(tokenizer.encode(full_prompt)) + len(tokenizer.encode(text))
            return text, tokens

        except Exception as exc:
            logger.error("HF Transformers generation failed: %s", exc)
            # Fallback to HF Inference API
            from engine.services.hf import hf_service

            combined = f"{system}\n\n{prompt}"
            text = str(hf_service.text_generation(model_id=model_id, prompt=combined, max_new_tokens=max_tokens))
            return text, len(combined.split()) + len(text.split())

    def _parse_json(self, text: str) -> dict[str, Any] | None:
        """Extract JSON from model output, handling markdown fences and extra text."""
        # Strip markdown code fences
        cleaned = text.strip()
        if cleaned.startswith("```"):
            lines = cleaned.split("\n")
            # Remove first and last lines (fences)
            lines = lines[1:] if lines[0].startswith("```") else lines
            if lines and lines[-1].strip() == "```":
                lines = lines[:-1]
            cleaned = "\n".join(lines).strip()

        # Try direct parse
        try:
            return json.loads(cleaned)
        except json.JSONDecodeError:
            pass

        # Try to find JSON object in the text
        start = cleaned.find("{")
        end = cleaned.rfind("}")
        if start >= 0 and end > start:
            try:
                return json.loads(cleaned[start : end + 1])
            except json.JSONDecodeError:
                pass

        # Try to find JSON array
        start = cleaned.find("[")
        end = cleaned.rfind("]")
        if start >= 0 and end > start:
            try:
                arr = json.loads(cleaned[start : end + 1])
                if isinstance(arr, list) and arr:
                    return arr[0] if isinstance(arr[0], dict) else {"data": arr}
            except json.JSONDecodeError:
                pass

        return None

    async def _save_output(self, job: SynthesisJob, output_dir: Path) -> Path:
        """Save generated samples to file in the requested format."""
        cfg = job.config
        base_name = f"{cfg.job_id}_{cfg.name.replace(' ', '_').lower()}"

        # Strip _meta for output files
        clean_samples = []
        for s in job.samples:
            sample = {k: v for k, v in s.items() if k != "_meta"}
            clean_samples.append(sample)

        if cfg.output_format == OutputFormat.CSV:
            path = output_dir / f"{base_name}.csv"
            if clean_samples:
                keys = [c["name"] for c in cfg.schema] if cfg.schema else list(clean_samples[0].keys())
                with open(path, "w", newline="", encoding="utf-8") as f:
                    writer = csv.DictWriter(f, fieldnames=keys, extrasaction="ignore")
                    writer.writeheader()
                    for row in clean_samples:
                        flat = {k: json.dumps(v) if isinstance(v, (dict, list)) else str(v) for k, v in row.items()}
                        writer.writerow(flat)
        elif cfg.output_format == OutputFormat.DELTA:
            # Save as JSONL with delta-compatible structure (partitioned by timestamp)
            path = output_dir / f"{base_name}.delta.jsonl"
            with open(path, "w", encoding="utf-8") as f:
                for s in clean_samples:
                    f.write(json.dumps(s, ensure_ascii=False) + "\n")
        else:
            # JSONL, Alpaca, ChatML, ShareGPT — all JSONL
            ext = ".jsonl"
            path = output_dir / f"{base_name}{ext}"
            with open(path, "w", encoding="utf-8") as f:
                for s in clean_samples:
                    f.write(json.dumps(s, ensure_ascii=False) + "\n")

        logger.info("Saved %d samples to %s", len(clean_samples), path)
        return path

    async def _store_in_qdrant(self, job: SynthesisJob) -> None:
        """Optionally store samples as embeddings in Qdrant for RAG use."""
        try:
            from engine.services.hf import hf_service
            from engine.services.qdrant import qdrant_service

            texts = []
            payloads = []
            for i, sample in enumerate(job.samples):
                # Create a text representation for embedding
                text = json.dumps({k: v for k, v in sample.items() if k != "_meta"}, ensure_ascii=False)
                texts.append(text)
                payloads.append(
                    {
                        "job_id": job.config.job_id,
                        "sample_index": i,
                        "format": job.config.output_format.value,
                        "source": job.config.source.value,
                        "model": job.config.model,
                    }
                )

            # Generate embeddings
            embeddings = hf_service.text_embedding(
                model_id="sentence-transformers/all-MiniLM-L6-v2",
                text=texts,
            )

            # Build point dicts matching QdrantService.upsert signature
            points = [
                {
                    "id": f"{job.config.job_id}-{i}",
                    "vector": embeddings[i],
                    "payload": payloads[i],
                }
                for i in range(len(texts))
            ]

            await qdrant_service.upsert(points=points)
            logger.info("Stored %d embeddings in Qdrant collection", len(points))
        except Exception as exc:
            logger.warning("Failed to store in Qdrant: %s", exc)


synthesis_service = SynthesisService()
