from __future__ import annotations

import logging
from typing import Any

from huggingface_hub import HfApi, InferenceClient

from engine.config import settings

logger = logging.getLogger("engine.hf")


class HuggingFaceService:
    """HuggingFace Hub + Inference integration."""

    def __init__(self) -> None:
        self._api: HfApi | None = None
        self._inference: InferenceClient | None = None
        self._token: str = settings.hf_token or ""
        self._local_model: Any = None
        self._local_available: bool | None = None  # None = not checked yet

    def set_token(self, token: str) -> None:
        """Update the API token (e.g. loaded from database)."""
        if token != self._token:
            self._token = token
            self._api = None  # force re-init
            self._inference = None

    @property
    def api(self) -> HfApi:
        if self._api is None:
            self._api = HfApi(token=self._token or None)
        return self._api

    @property
    def inference(self) -> InferenceClient:
        if self._inference is None:
            self._inference = InferenceClient(token=self._token or None)
        return self._inference

    @property
    def has_token(self) -> bool:
        return bool(self._token)

    def search_models(
        self,
        query: str = "",
        pipeline_tag: str | None = None,
        library: str | None = None,
        sort: str = "downloads",
        limit: int = 20,
    ) -> list[dict[str, Any]]:
        models = self.api.list_models(
            search=query or None,
            pipeline_tag=pipeline_tag,
            library=library,
            sort=sort,
            limit=limit,
        )
        return [
            {
                "id": m.id,
                "pipeline_tag": m.pipeline_tag,
                "downloads": m.downloads,
                "likes": m.likes,
                "tags": m.tags,
            }
            for m in models
        ]

    def search_datasets(
        self,
        query: str = "",
        sort: str = "downloads",
        limit: int = 20,
    ) -> list[dict[str, Any]]:
        """Search datasets and enrich with size from the individual dataset API."""
        from concurrent.futures import ThreadPoolExecutor, as_completed

        import httpx as _httpx

        datasets = list(self.api.list_datasets(search=query or None, sort=sort, limit=limit))

        results = [
            {
                "id": d.id,
                "author": d.author,
                "downloads": d.downloads,
                "likes": d.likes,
                "tags": d.tags,
                "private": d.private,
                "last_modified": str(d.last_modified) if d.last_modified else None,
                "created_at": str(d.created_at) if d.created_at else None,
                "size_bytes": None,
                "description": None,
            }
            for d in datasets
        ]

        # Batch-fetch usedStorage + description from individual dataset endpoints
        def _fetch_size(dataset_id: str) -> tuple[str, int | None, str | None]:
            try:
                resp = _httpx.get(
                    f"https://huggingface.co/api/datasets/{dataset_id}",
                    timeout=8,
                    headers={"Authorization": f"Bearer {self._token}"} if self._token else {},
                )
                if resp.status_code == 200:
                    data = resp.json()
                    return dataset_id, data.get("usedStorage"), data.get("description")
            except Exception:
                pass
            return dataset_id, None, None

        # Limit to 15 concurrent fetches to avoid hammering the API
        with ThreadPoolExecutor(max_workers=min(15, len(results))) as pool:
            futures = {pool.submit(_fetch_size, r["id"]): r for r in results}
            for fut in as_completed(futures):
                ds_id, size, desc = fut.result()
                entry = futures[fut]
                entry["size_bytes"] = size
                entry["description"] = desc

        return results

    def get_dataset_info(self, dataset_id: str) -> dict[str, Any]:
        """Get detailed info for a specific dataset."""
        info = self.api.dataset_info(dataset_id)
        return {
            "id": info.id,
            "author": info.author,
            "downloads": info.downloads,
            "likes": info.likes,
            "tags": info.tags,
            "description": info.description if hasattr(info, "description") else None,
            "citation": info.citation if hasattr(info, "citation") else None,
            "card_data": info.card_data if hasattr(info, "card_data") else None,
            "created_at": str(info.created_at) if info.created_at else None,
            "last_modified": str(info.last_modified) if info.last_modified else None,
            "private": info.private,
        }

    def download_dataset(self, dataset_id: str, config: str | None = None, split: str | None = None) -> dict[str, Any]:
        """Download a dataset to the default HuggingFace cache directory and return info about it.

        Handles multiple failure modes:
        - trust_remote_code deprecation → never pass it
        - webdataset/script-based datasets → fallback to snapshot_download
        - missing configs → try without config
        - auth → uses stored token if available
        """
        from datasets import get_dataset_config_names, load_dataset
        from huggingface_hub import constants as hf_constants
        from huggingface_hub import scan_cache_dir, snapshot_download

        cache_path = str(hf_constants.HF_HUB_CACHE)
        token = self._token or None

        # Get available configs (best effort)
        configs: list[str] = []
        try:
            configs = get_dataset_config_names(dataset_id, token=token)
        except Exception:
            configs = []

        selected_config = config if config and config in configs else (configs[0] if configs else None)

        # Attempt 1: load_dataset (standard parquet/csv/json datasets)
        ds = None
        load_error: str | None = None
        for attempt_config in [selected_config, None]:
            try:
                kwargs: dict[str, Any] = {"token": token}
                if attempt_config and attempt_config != "default":
                    kwargs["name"] = attempt_config
                if split:
                    kwargs["split"] = split
                ds = load_dataset(dataset_id, **kwargs)
                selected_config = attempt_config
                break
            except Exception as exc:
                load_error = str(exc)
                logger.warning("load_dataset(%s, config=%s) failed: %s", dataset_id, attempt_config, exc)
                continue

        # Attempt 2: snapshot_download — just download the raw repo files
        if ds is None:
            logger.info("Falling back to snapshot_download for %s", dataset_id)
            try:
                local_dir = snapshot_download(
                    repo_id=dataset_id,
                    repo_type="dataset",
                    token=token,
                )
                # Return minimal info from the snapshot
                size_bytes = 0
                try:
                    cache_info = scan_cache_dir()
                    for repo in cache_info.repos:
                        if dataset_id.replace("/", "--") in str(repo.repo_path):
                            size_bytes = repo.size_on_disk
                            break
                except Exception:
                    pass

                return {
                    "dataset_id": dataset_id,
                    "config": selected_config,
                    "configs_available": configs,
                    "splits": {},
                    "num_rows": 0,
                    "columns": [],
                    "features": {},
                    "cache_path": local_dir,
                    "size_bytes": size_bytes,
                    "note": "Downloaded as raw snapshot — dataset could not be parsed as structured data",
                }
            except Exception as snap_exc:
                raise RuntimeError(f"Failed to download dataset '{dataset_id}'. load_dataset error: {load_error}. snapshot_download error: {snap_exc}") from snap_exc

        # Extract info from the loaded dataset
        if split:
            num_rows = len(ds)
            columns = ds.column_names
            features = {k: str(v) for k, v in ds.features.items()}
            splits_info = {split: len(ds)}
        else:
            # ds is a DatasetDict
            splits_info = {s: len(ds[s]) for s in ds}
            num_rows = sum(splits_info.values())
            first_split = list(ds.keys())[0]
            columns = ds[first_split].column_names
            features = {k: str(v) for k, v in ds[first_split].features.items()}

        # Estimate size on disk
        size_bytes = 0
        try:
            cache_info = scan_cache_dir()
            for repo in cache_info.repos:
                if dataset_id.replace("/", "--") in str(repo.repo_path):
                    size_bytes = repo.size_on_disk
                    break
        except Exception:
            pass

        return {
            "dataset_id": dataset_id,
            "config": selected_config,
            "configs_available": configs,
            "splits": splits_info,
            "num_rows": num_rows,
            "columns": columns,
            "features": features,
            "cache_path": cache_path,
            "size_bytes": size_bytes,
        }

    def get_dataset_preview(self, dataset_id: str, config: str | None = None, split: str = "train", rows: int = 20) -> dict[str, Any]:
        """Load a dataset from cache and return preview rows."""
        from datasets import load_dataset

        kwargs: dict[str, Any] = {"split": split, "token": self._token or None}
        if config and config != "default":
            kwargs["name"] = config

        ds = load_dataset(dataset_id, **kwargs)
        preview = [ds[i] for i in range(min(rows, len(ds)))]
        columns = ds.column_names
        features = {k: str(v) for k, v in ds.features.items()}

        return {
            "dataset_id": dataset_id,
            "split": split,
            "total_rows": len(ds),
            "columns": columns,
            "features": features,
            "rows": preview,
        }

    def get_model_info(self, model_id: str) -> dict[str, Any]:
        info = self.api.model_info(model_id)
        return {
            "id": info.id,
            "pipeline_tag": info.pipeline_tag,
            "downloads": info.downloads,
            "likes": info.likes,
            "tags": info.tags,
            "library_name": info.library_name,
            "created_at": str(info.created_at) if info.created_at else None,
            "last_modified": str(info.last_modified) if info.last_modified else None,
        }

    async def run_inference(self, model_id: str, inputs: Any, parameters: dict[str, Any] | None = None) -> Any:
        """Run inference on a HuggingFace hosted model."""
        return self.inference.post(
            model=model_id,
            json={"inputs": inputs, "parameters": parameters or {}},
        )

    def text_generation(self, model_id: str, prompt: str, **kwargs: Any) -> str:
        result = self.inference.text_generation(prompt, model=model_id, **kwargs)
        return result

    def _get_local_model(self, model_id: str) -> Any:
        """Lazily load sentence-transformers model for local embedding."""
        if self._local_available is False:
            return None
        if self._local_model is not None:
            return self._local_model
        try:
            from sentence_transformers import SentenceTransformer

            self._local_model = SentenceTransformer(model_id)
            self._local_available = True
            logger.info("Loaded local embedding model: %s", model_id)
            return self._local_model
        except ImportError:
            logger.warning("sentence-transformers not installed; local embeddings unavailable")
            self._local_available = False
            return None
        except Exception as exc:
            logger.warning("Failed to load local embedding model %s: %s", model_id, exc)
            self._local_available = False
            return None

    def text_embedding(self, model_id: str, text: str | list[str]) -> list[list[float]]:
        """Get embeddings — tries local sentence-transformers first, then HF API."""
        # Strategy 1: Local model (no token needed, faster)
        local_model = self._get_local_model(model_id)
        if local_model is not None:
            try:
                texts = [text] if isinstance(text, str) else text
                embeddings = local_model.encode(texts, convert_to_numpy=True)
                return embeddings.tolist()
            except Exception as exc:
                logger.warning("Local embedding failed, trying HF API: %s", exc)

        # Strategy 2: HF Inference API (requires token)
        if not self.has_token:
            logger.warning(
                "No HF_TOKEN set and local sentence-transformers unavailable; cannot generate embeddings. Set HF_TOKEN in .env or install sentence-transformers."
            )
            return []

        try:
            result = self.inference.feature_extraction(text, model=model_id)
            if isinstance(result[0], float):
                return [result]
            return result
        except Exception as exc:
            logger.error("HF Inference API embedding failed: %s", exc)
            return []


hf_service = HuggingFaceService()
