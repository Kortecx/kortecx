from __future__ import annotations

import logging
from typing import Any

logger = logging.getLogger("engine.chains")


class ChainService:
    """LangChain-based orchestration pipelines."""

    def run_chain(self, chain_type: str, inputs: dict[str, Any], config: dict[str, Any] | None = None) -> Any:
        """Run a LangChain chain by type."""
        if chain_type == "qa":
            return self._qa_chain(inputs, config or {})
        elif chain_type == "summarize":
            return self._summarize_chain(inputs, config or {})
        elif chain_type == "rag":
            return self._rag_chain(inputs, config or {})
        raise ValueError(f"Unknown chain type: {chain_type}")

    def _qa_chain(self, inputs: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
        from langchain.chains import RetrievalQA
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
        from langchain_huggingface import HuggingFaceEmbeddings, HuggingFacePipeline
        from langchain_community.vectorstores import Qdrant as LangchainQdrant
        from langchain.chains import RetrievalQA
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
