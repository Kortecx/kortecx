"""PR-B2 vision chaining — ``chat(image=...)`` binds kx/recipes/vision. Pure, no server."""

from typing import List, Optional

import pytest

from kortecx import KxClient, KxUsage
from kortecx.content import PutResult
from kortecx.recipes import RecipeForm, RecipeFormField
from kortecx.run import Result


class _VisionFake(KxClient):
    """A KxClient whose put/form/invoke are doubled so the test captures the wire shape
    ``chat(image=...)`` produces without a server (``__init__`` is bypassed so nothing
    dials)."""

    def __init__(self, fields: Optional[List[RecipeFormField]], default_model: str = "") -> None:
        self.put_calls: List[bytes] = []
        self.invoked: Optional[tuple] = None
        self._fields = fields
        self.default_model = default_model

    def put_content(self, payload: bytes, *, media_type: str = "", filename: str = "") -> PutResult:
        self.put_calls.append(payload)
        return PutResult(content_ref="ab" * 32, size=len(payload), deduplicated=False)

    def get_recipe_form(self, handle: str) -> RecipeForm:
        if self._fields is None:
            raise RuntimeError("kx/recipes/vision not provisioned")
        return RecipeForm(handle=handle, fields=self._fields)

    def invoke(  # type: ignore[override]
        self, handle, args, *, wait=False, timeout=120.0, stream=False, out=None, context=None
    ):
        self.invoked = (handle, args)
        return Result(
            instance_id="",
            terminal_mote_id="",
            state="COMMITTED",
            result_ref=None,
            payload=b"a cat",
        )


def _vision_form(allowed: List[str]) -> List[RecipeFormField]:
    return [
        RecipeFormField(name="prompt", type="str", required=True, max_len=4096, allowed=[]),
        RecipeFormField(name="image_ref", type="bytes", required=True, max_len=64, allowed=[]),
        RecipeFormField(name="model", type="enum", required=True, max_len=None, allowed=allowed),
    ]


def test_chat_image_bytes_binds_vision_recipe() -> None:
    c = _VisionFake(_vision_form(["gemma3:12b"]))
    out = c.chat("what is in this image?", image=b"\x89PNG")
    assert out == "a cat"
    assert c.put_calls == [b"\x89PNG"]
    handle, args = c.invoked
    assert handle == "kx/recipes/vision"
    assert args == {
        "image_ref": "ab" * 32,
        "prompt": "what is in this image?",
        "model": "gemma3:12b",
    }


def test_chat_image_ref_passes_through_without_upload() -> None:
    c = _VisionFake(_vision_form(["m"]))
    c.chat("ocr please", image={"ref": "cd" * 32})
    assert c.put_calls == []
    assert c.invoked[1]["image_ref"] == "cd" * 32


def test_chat_prefers_default_model_when_legal() -> None:
    c = _VisionFake(_vision_form(["a", "b", "gemma3:12b"]), default_model="gemma3:12b")
    c.chat("hi", image=b"\x01")
    assert c.invoked[1]["model"] == "gemma3:12b"


def test_dataset_and_image_binds_vision_rag() -> None:
    # RC4b: image + dataset now binds kx/recipes/vision-rag (the VLM answers about the
    # image WHILE grounded on the dataset's retrieved text) — no longer a usage error.
    c = _VisionFake(_vision_form(["m"]))
    out = c.chat("describe", image=b"\x01", dataset="docs", k=3)
    assert out == "a cat"
    handle, args = c.invoked
    assert handle == "kx/recipes/vision-rag"
    assert args["image_ref"] == "ab" * 32
    assert args["dataset"] == "docs"
    assert args["k"] == 3


def test_dataset_and_image_honest_degrades_when_vision_rag_absent() -> None:
    # No vision-rag recipe (no image-capable model / non-hnsw serve) ⇒ a clear KxUsage,
    # never a silent drop of the image or the dataset (GR15).
    c = _VisionFake(None)
    with pytest.raises(KxUsage):
        c.chat("hi", image=b"\x01", dataset="docs")


def test_no_vision_model_honest_degrades_to_error() -> None:
    c = _VisionFake(None)  # get_recipe_form raises ⇒ no image-capable model
    with pytest.raises(KxUsage):
        c.chat("hi", image=b"\x01")


def test_plain_chat_unaffected() -> None:
    c = _VisionFake(_vision_form(["m"]))
    out = c.chat("hello")
    assert out == "a cat"
    assert c.put_calls == []
    assert c.invoked[0] == "kx/recipes/chat"
