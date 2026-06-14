"""UI-2 recipe-form view — a recipe's variable free-params (``GetRecipeForm``).

Kept in its own module so ``types.py`` stays a thin aggregator. The param type
renders to a stable lowercase name; an out-of-range value (a future
``RecipeParamType``) renders ``"unspecified"`` — never a crash, never a silent
mislabel (mirrors the TS ``recipeParamTypeName``).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Optional

from .v1 import gateway_pb2 as _g

_PARAM_TYPE_NAMES: dict[int, str] = {
    _g.RECIPE_PARAM_TYPE_STR: "str",
    _g.RECIPE_PARAM_TYPE_INT: "int",
    _g.RECIPE_PARAM_TYPE_BOOL: "bool",
    _g.RECIPE_PARAM_TYPE_BYTES: "bytes",
    _g.RECIPE_PARAM_TYPE_ENUM: "enum",
}


def recipe_param_type_name(t: int) -> str:
    """Map a ``RecipeParamType`` discriminant to a stable name (``"unspecified"``
    for UNSPECIFIED(0) or any future value)."""
    return _PARAM_TYPE_NAMES.get(t, "unspecified")


@dataclass(frozen=True)
class RecipeFormField:
    """One free-param a recipe requires (the unit a form renders as an input)."""

    name: str
    type: str  # one of: str | int | bool | bytes | enum | unspecified
    required: bool
    max_len: Optional[int]  # for str / bytes (else None)
    allowed: List[str]  # for enum (else empty)

    @classmethod
    def from_proto(cls, f: "_g.RecipeFormField") -> "RecipeFormField":
        return cls(
            name=f.name,
            type=recipe_param_type_name(f.type),
            required=f.required,
            max_len=f.max_len if f.HasField("max_len") else None,
            allowed=list(f.allowed),
        )


@dataclass(frozen=True)
class RecipeForm:
    """A recipe's input FORM: its handle + the ordered variable free-param fields."""

    handle: str
    fields: List[RecipeFormField]

    @classmethod
    def from_proto(cls, r: "_g.GetRecipeFormResponse") -> "RecipeForm":
        return cls(
            handle=r.handle,
            fields=[RecipeFormField.from_proto(f) for f in r.fields],
        )


# Display-layer aliases (D136): the user-facing name is **Blueprint** — a
# reusable, shareable workflow template. The WIRE stays the frozen `recipe`
# vocabulary (``ListRecipes``/``GetRecipeForm``, ``kx/recipes/*`` handles), so
# these are pure additive aliases; nothing is renamed or deprecated.
BlueprintForm = RecipeForm
BlueprintFormField = RecipeFormField
blueprint_param_type_name = recipe_param_type_name


@dataclass(frozen=True)
class RecipeInfo:
    """One catalog entry of ``ListRecipes`` (PR-2.1): the Invoke handle plus the
    published workflow fingerprint a bound run registers under — the join key
    for labeling durable ``RunSummary`` rows. PR-4 Batch D adds the ADVISORY
    metadata (description / tags / version) — display/discovery ONLY, never
    identity, never enforcement. ``recipe_fingerprint`` / metadata are empty
    when the gateway predates the field."""

    handle: str
    recipe_fingerprint: str  # hex; "" if unknown
    description: str = ""  # advisory; never parsed for enforcement
    tags: List[str] = field(default_factory=list)  # advisory discovery tags
    version: str = ""  # advisory published version label; "" if unversioned

    @classmethod
    def from_proto(cls, r: "_g.RecipeSummary") -> "RecipeInfo":
        return cls(
            handle=r.handle,
            recipe_fingerprint=r.recipe_fingerprint.hex(),
            description=r.description,
            tags=list(r.tags),
            version=r.version,
        )

    def to_dict(self) -> dict:
        return {
            "handle": self.handle,
            "recipe_fingerprint": self.recipe_fingerprint,
            "description": self.description,
            "tags": list(self.tags),
            "version": self.version,
        }


@dataclass(frozen=True)
class ScoredRecipe:
    """One ranked ``SearchRecipes`` hit (PR-4 Batch D): the matched recipe plus
    its advisory rank in integer basis points (0..=10000). SN-8: ``score_bp`` is
    DISPLAY-ONLY — a search SURFACES a recipe, never invokes one (``Invoke``
    stays the authorization gate)."""

    recipe: RecipeInfo
    score_bp: int  # 0..=10000; never a float

    @classmethod
    def from_proto(cls, s: "_g.ScoredRecipe") -> "ScoredRecipe":
        return cls(recipe=RecipeInfo.from_proto(s.recipe), score_bp=s.score_bp)

    def to_dict(self) -> dict:
        return {"recipe": self.recipe.to_dict(), "score_bp": self.score_bp}
