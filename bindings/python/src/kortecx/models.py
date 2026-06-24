"""Batch A model-discovery view — one ``ListModels`` entry.

Display/discovery ONLY (SN-8): model *selection* stays a recipe ENUM free-param
validated server-side at binding; nothing here authorizes a model route. An
FFI-free gateway answers with an EMPTY list (honest, not an error).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Tuple

from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class ModelSummary:
    """One discoverable model on the connected gateway."""

    model_id: str  # the id a recipe `model` ENUM free-param accepts
    modalities: Tuple[str, ...]  # display strings: "text" | "image" | "audio" | "video"
    description: str  # host-synthesized display prose — never identity
    serving: bool  # the PRIMARY/default serve route
    context_len: int  # the served context window in tokens
    loaded: bool = False  # POC-3: resident in RAM right now (live LRU residency)
    chat_handle: str = ""  # POC-3: the recipe handle to chat with THIS model

    @classmethod
    def from_proto(cls, m: "_g.ModelSummary") -> "ModelSummary":
        return cls(
            model_id=m.model_id,
            modalities=tuple(m.modalities),
            description=m.description,
            serving=m.serving,
            context_len=m.context_len,
            loaded=m.loaded,
            chat_handle=m.chat_handle,
        )


@dataclass(frozen=True)
class ModelLifecycleResult:
    """The outcome of a ``load_model`` / ``offload_model`` call (POC-3)."""

    model_id: str  # the model the op targeted
    loaded: bool  # residency AFTER the op (True after load, False after offload)
    was_resident: bool  # residency BEFORE the op (load: False ⇒ cold load)

    @classmethod
    def from_load(cls, r: "_g.LoadModelResponse") -> "ModelLifecycleResult":
        return cls(model_id=r.model_id, loaded=r.loaded, was_resident=r.was_resident)

    @classmethod
    def from_offload(cls, r: "_g.OffloadModelResponse") -> "ModelLifecycleResult":
        return cls(model_id=r.model_id, loaded=r.loaded, was_resident=r.was_resident)
