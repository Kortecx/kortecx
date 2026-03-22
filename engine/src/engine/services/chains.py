from __future__ import annotations

import hashlib
import logging
import threading
import time
from collections import OrderedDict
from typing import Any

logger = logging.getLogger("engine.chains")

_CACHE_MAX_SIZE = 128


class _PipelineCache:
    """Thread-safe LRU cache for pipeline results, keyed by hash of pipeline_type + input text."""

    def __init__(self, max_size: int = _CACHE_MAX_SIZE) -> None:
        self._max_size = max_size
        self._cache: OrderedDict[str, dict[str, Any]] = OrderedDict()
        self._lock = threading.Lock()
        self.hits = 0
        self.misses = 0

    @staticmethod
    def _make_key(pipeline_type: str, input_text: str) -> str:
        raw = f"{pipeline_type}:{input_text}"
        return hashlib.sha256(raw.encode("utf-8")).hexdigest()

    def get(self, pipeline_type: str, input_text: str) -> dict[str, Any] | None:
        key = self._make_key(pipeline_type, input_text)
        with self._lock:
            if key in self._cache:
                self._cache.move_to_end(key)
                self.hits += 1
                return self._cache[key]
            self.misses += 1
            return None

    def put(self, pipeline_type: str, input_text: str, result: dict[str, Any]) -> None:
        key = self._make_key(pipeline_type, input_text)
        with self._lock:
            if key in self._cache:
                self._cache.move_to_end(key)
            self._cache[key] = result
            while len(self._cache) > self._max_size:
                self._cache.popitem(last=False)

    def clear(self) -> None:
        with self._lock:
            self._cache.clear()
            self.hits = 0
            self.misses = 0

    @property
    def size(self) -> int:
        with self._lock:
            return len(self._cache)


def _count_tokens(text: str) -> int:
    """Approximate token count by splitting on whitespace."""
    return len(text.split()) if text else 0


def _add_token_counts(result: dict[str, Any], input_text: str) -> dict[str, Any]:
    """Add input/output token counts to a pipeline result dict."""
    output_text = ""
    for key in ("answer", "summary"):
        if key in result and isinstance(result[key], str):
            output_text = result[key]
            break

    result["token_counts"] = {
        "input_tokens": _count_tokens(input_text),
        "output_tokens": _count_tokens(output_text),
        "total_tokens": _count_tokens(input_text) + _count_tokens(output_text),
    }
    return result


class ChainService:
    """LangChain-based orchestration pipelines."""

    def __init__(self) -> None:
        self.cache = _PipelineCache()

    def run_chain(self, chain_type: str, inputs: dict[str, Any], config: dict[str, Any] | None = None) -> Any:
        """Run a LangChain chain by type.

        Results are cached by pipeline_type + input text. Token counts
        (approximate, whitespace-split) are included in every response.
        """
        input_text = inputs.get("question", "") or inputs.get("text", "")

        # Check cache
        cached = self.cache.get(chain_type, input_text)
        if cached is not None:
            logger.info("Cache hit for chain=%s (cache size=%d)", chain_type, self.cache.size)
            return {**cached, "cached": True}

        start = time.monotonic()

        if chain_type == "qa":
            result = self._qa_chain(inputs, config or {})
        elif chain_type == "summarize":
            result = self._summarize_chain(inputs, config or {})
        elif chain_type == "rag":
            result = self._rag_chain(inputs, config or {})
        else:
            raise ValueError(f"Unknown chain type: {chain_type}")

        duration_ms = int((time.monotonic() - start) * 1000)
        result["duration_ms"] = duration_ms
        result["cached"] = False

        # Add token counts
        _add_token_counts(result, input_text)

        # Store in cache
        self.cache.put(chain_type, input_text, result)

        logger.info(
            "Chain %s completed in %dms (tokens: %d in / %d out)",
            chain_type,
            duration_ms,
            result["token_counts"]["input_tokens"],
            result["token_counts"]["output_tokens"],
        )
        return result

    def _qa_chain(self, inputs: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
        from langchain_huggingface import HuggingFacePipeline

        model_id = config.get("model_id", "google/flan-t5-base")
        llm = HuggingFacePipeline.from_model_id(model_id=model_id, task="text2text-generation")

        return {"answer": llm.invoke(inputs.get("question", "")), "model": model_id}

    def _summarize_chain(self, inputs: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
        from langchain_huggingface import HuggingFacePipeline

        model_id = config.get("model_id", "facebook/bart-large-cnn")
        llm = HuggingFacePipeline.from_model_id(model_id=model_id, task="summarization")

        text = inputs.get("text", "")
        return {"summary": llm.invoke(text), "model": model_id}

    def _rag_chain(self, inputs: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
        from langchain.chains import RetrievalQA
        from langchain_community.vectorstores import Qdrant as LangchainQdrant
        from langchain_huggingface import HuggingFaceEmbeddings, HuggingFacePipeline
        from qdrant_client import QdrantClient

        from engine.config import settings

        model_id = config.get("model_id", "google/flan-t5-base")
        embedding_model = config.get("embedding_model", "sentence-transformers/all-MiniLM-L6-v2")
        collection = config.get("collection", settings.qdrant_collection)

        embeddings = HuggingFaceEmbeddings(model_name=embedding_model)
        client = QdrantClient(url=settings.qdrant_url)

        vectorstore = LangchainQdrant(client=client, collection_name=collection, embeddings=embeddings)
        llm = HuggingFacePipeline.from_model_id(model_id=model_id, task="text2text-generation")

        qa = RetrievalQA.from_chain_type(llm=llm, retriever=vectorstore.as_retriever())
        result = qa.invoke({"query": inputs.get("question", "")})

        return {"answer": result.get("result", ""), "model": model_id}


chain_service = ChainService()

