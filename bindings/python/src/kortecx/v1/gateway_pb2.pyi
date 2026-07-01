from kortecx.v1 import coordinator_pb2 as _coordinator_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Iterable as _Iterable, Mapping as _Mapping, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class MoteSnapshotState(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MOTE_SNAPSHOT_STATE_UNSPECIFIED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_PENDING: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_SCHEDULED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_COMMITTED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_FAILED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_REPUDIATED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_INCONSISTENT: _ClassVar[MoteSnapshotState]

class PromotionState(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    PROMOTION_STATE_UNSPECIFIED: _ClassVar[PromotionState]
    PROMOTION_STATE_NOT_APPLICABLE: _ClassVar[PromotionState]
    PROMOTION_STATE_UNPROMOTED: _ClassVar[PromotionState]
    PROMOTION_STATE_PROMOTED: _ClassVar[PromotionState]

class MoteAnomaly(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MOTE_ANOMALY_UNSPECIFIED: _ClassVar[MoteAnomaly]
    MOTE_ANOMALY_EFFECT_STAGED_THEN_REPUDIATED_NO_COMMITTED: _ClassVar[MoteAnomaly]
    MOTE_ANOMALY_QUARANTINED_AT_LEAST_ONCE_EFFECT: _ClassVar[MoteAnomaly]

class RecipeParamType(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    RECIPE_PARAM_TYPE_UNSPECIFIED: _ClassVar[RecipeParamType]
    RECIPE_PARAM_TYPE_STR: _ClassVar[RecipeParamType]
    RECIPE_PARAM_TYPE_INT: _ClassVar[RecipeParamType]
    RECIPE_PARAM_TYPE_BOOL: _ClassVar[RecipeParamType]
    RECIPE_PARAM_TYPE_BYTES: _ClassVar[RecipeParamType]
    RECIPE_PARAM_TYPE_ENUM: _ClassVar[RecipeParamType]

class RetrievalMode(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    RETRIEVAL_MODE_UNSPECIFIED: _ClassVar[RetrievalMode]
    RETRIEVAL_MODE_DENSE: _ClassVar[RetrievalMode]
    RETRIEVAL_MODE_HYBRID: _ClassVar[RetrievalMode]

class MemoryKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MEMORY_KIND_UNSPECIFIED: _ClassVar[MemoryKind]
    MEMORY_KIND_SEMANTIC: _ClassVar[MemoryKind]
    MEMORY_KIND_EPISODIC: _ClassVar[MemoryKind]

class LowerVerdict(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    LOWER_VERDICT_UNSPECIFIED: _ClassVar[LowerVerdict]
    LOWER_VERDICT_UNAVAILABLE: _ClassVar[LowerVerdict]
    LOWER_VERDICT_WOULD_LOWER: _ClassVar[LowerVerdict]
    LOWER_VERDICT_REFUSED: _ClassVar[LowerVerdict]

class WorkflowStepKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    WORKFLOW_STEP_KIND_UNSPECIFIED: _ClassVar[WorkflowStepKind]
    WORKFLOW_STEP_KIND_PURE: _ClassVar[WorkflowStepKind]
    WORKFLOW_STEP_KIND_MODEL: _ClassVar[WorkflowStepKind]
    WORKFLOW_STEP_KIND_EXEC: _ClassVar[WorkflowStepKind]
    WORKFLOW_STEP_KIND_TOOL: _ClassVar[WorkflowStepKind]

class WorkflowExecutionMode(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    WORKFLOW_EXECUTION_MODE_UNSPECIFIED: _ClassVar[WorkflowExecutionMode]
    WORKFLOW_EXECUTION_MODE_FROZEN: _ClassVar[WorkflowExecutionMode]
    WORKFLOW_EXECUTION_MODE_DYNAMIC: _ClassVar[WorkflowExecutionMode]

class FeedbackRating(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    FEEDBACK_RATING_UNSPECIFIED: _ClassVar[FeedbackRating]
    FEEDBACK_RATING_UP: _ClassVar[FeedbackRating]
    FEEDBACK_RATING_DOWN: _ClassVar[FeedbackRating]

class TriggerKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    TRIGGER_KIND_UNSPECIFIED: _ClassVar[TriggerKind]
    WEBHOOK: _ClassVar[TriggerKind]
    CRON: _ClassVar[TriggerKind]
    GRPC: _ClassVar[TriggerKind]

class TriggerAuth(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    TRIGGER_AUTH_UNSPECIFIED: _ClassVar[TriggerAuth]
    NONE: _ClassVar[TriggerAuth]
    HMAC_SHA256: _ClassVar[TriggerAuth]
    BEARER: _ClassVar[TriggerAuth]
MOTE_SNAPSHOT_STATE_UNSPECIFIED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_PENDING: MoteSnapshotState
MOTE_SNAPSHOT_STATE_SCHEDULED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_COMMITTED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_FAILED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_REPUDIATED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_INCONSISTENT: MoteSnapshotState
PROMOTION_STATE_UNSPECIFIED: PromotionState
PROMOTION_STATE_NOT_APPLICABLE: PromotionState
PROMOTION_STATE_UNPROMOTED: PromotionState
PROMOTION_STATE_PROMOTED: PromotionState
MOTE_ANOMALY_UNSPECIFIED: MoteAnomaly
MOTE_ANOMALY_EFFECT_STAGED_THEN_REPUDIATED_NO_COMMITTED: MoteAnomaly
MOTE_ANOMALY_QUARANTINED_AT_LEAST_ONCE_EFFECT: MoteAnomaly
RECIPE_PARAM_TYPE_UNSPECIFIED: RecipeParamType
RECIPE_PARAM_TYPE_STR: RecipeParamType
RECIPE_PARAM_TYPE_INT: RecipeParamType
RECIPE_PARAM_TYPE_BOOL: RecipeParamType
RECIPE_PARAM_TYPE_BYTES: RecipeParamType
RECIPE_PARAM_TYPE_ENUM: RecipeParamType
RETRIEVAL_MODE_UNSPECIFIED: RetrievalMode
RETRIEVAL_MODE_DENSE: RetrievalMode
RETRIEVAL_MODE_HYBRID: RetrievalMode
MEMORY_KIND_UNSPECIFIED: MemoryKind
MEMORY_KIND_SEMANTIC: MemoryKind
MEMORY_KIND_EPISODIC: MemoryKind
LOWER_VERDICT_UNSPECIFIED: LowerVerdict
LOWER_VERDICT_UNAVAILABLE: LowerVerdict
LOWER_VERDICT_WOULD_LOWER: LowerVerdict
LOWER_VERDICT_REFUSED: LowerVerdict
WORKFLOW_STEP_KIND_UNSPECIFIED: WorkflowStepKind
WORKFLOW_STEP_KIND_PURE: WorkflowStepKind
WORKFLOW_STEP_KIND_MODEL: WorkflowStepKind
WORKFLOW_STEP_KIND_EXEC: WorkflowStepKind
WORKFLOW_STEP_KIND_TOOL: WorkflowStepKind
WORKFLOW_EXECUTION_MODE_UNSPECIFIED: WorkflowExecutionMode
WORKFLOW_EXECUTION_MODE_FROZEN: WorkflowExecutionMode
WORKFLOW_EXECUTION_MODE_DYNAMIC: WorkflowExecutionMode
FEEDBACK_RATING_UNSPECIFIED: FeedbackRating
FEEDBACK_RATING_UP: FeedbackRating
FEEDBACK_RATING_DOWN: FeedbackRating
TRIGGER_KIND_UNSPECIFIED: TriggerKind
WEBHOOK: TriggerKind
CRON: TriggerKind
GRPC: TriggerKind
TRIGGER_AUTH_UNSPECIFIED: TriggerAuth
NONE: TriggerAuth
HMAC_SHA256: TriggerAuth
BEARER: TriggerAuth

class SubmitRunRequest(_message.Message):
    __slots__ = ("recipe_fingerprint", "motes")
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    MOTES_FIELD_NUMBER: _ClassVar[int]
    recipe_fingerprint: bytes
    motes: _containers.RepeatedCompositeFieldContainer[SubmitMoteSpec]
    def __init__(self, recipe_fingerprint: _Optional[bytes] = ..., motes: _Optional[_Iterable[_Union[SubmitMoteSpec, _Mapping]]] = ...) -> None: ...

class SubmitMoteSpec(_message.Message):
    __slots__ = ("mote", "warrant", "accept_at_least_once", "react_seed")
    MOTE_FIELD_NUMBER: _ClassVar[int]
    WARRANT_FIELD_NUMBER: _ClassVar[int]
    ACCEPT_AT_LEAST_ONCE_FIELD_NUMBER: _ClassVar[int]
    REACT_SEED_FIELD_NUMBER: _ClassVar[int]
    mote: _coordinator_pb2.Mote
    warrant: _coordinator_pb2.WarrantSpec
    accept_at_least_once: bool
    react_seed: bool
    def __init__(self, mote: _Optional[_Union[_coordinator_pb2.Mote, _Mapping]] = ..., warrant: _Optional[_Union[_coordinator_pb2.WarrantSpec, _Mapping]] = ..., accept_at_least_once: bool = ..., react_seed: bool = ...) -> None: ...

class RunHandle(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ...) -> None: ...

class InvokeRequest(_message.Message):
    __slots__ = ("handle", "args", "context_bundles", "context_refs")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    ARGS_FIELD_NUMBER: _ClassVar[int]
    CONTEXT_BUNDLES_FIELD_NUMBER: _ClassVar[int]
    CONTEXT_REFS_FIELD_NUMBER: _ClassVar[int]
    handle: str
    args: bytes
    context_bundles: _containers.RepeatedScalarFieldContainer[str]
    context_refs: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, handle: _Optional[str] = ..., args: _Optional[bytes] = ..., context_bundles: _Optional[_Iterable[str]] = ..., context_refs: _Optional[_Iterable[str]] = ...) -> None: ...

class InvokeResponse(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint", "terminal_mote_id", "react_chain_salt")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    TERMINAL_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    REACT_CHAIN_SALT_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    terminal_mote_id: bytes
    react_chain_salt: bytes
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ..., terminal_mote_id: _Optional[bytes] = ..., react_chain_salt: _Optional[bytes] = ...) -> None: ...

class GetProjectionRequest(_message.Message):
    __slots__ = ("instance_id", "at_seq")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    AT_SEQ_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    at_seq: int
    def __init__(self, instance_id: _Optional[bytes] = ..., at_seq: _Optional[int] = ...) -> None: ...

class ProjectionView(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint", "current_seq", "motes")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    CURRENT_SEQ_FIELD_NUMBER: _ClassVar[int]
    MOTES_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    current_seq: int
    motes: _containers.RepeatedCompositeFieldContainer[MoteSnapshot]
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ..., current_seq: _Optional[int] = ..., motes: _Optional[_Iterable[_Union[MoteSnapshot, _Mapping]]] = ...) -> None: ...

class MoteSnapshot(_message.Message):
    __slots__ = ("mote_id", "state", "nd_class", "promotion", "result_ref", "warrant_ref", "mote_def_hash", "committed_seq", "parents", "verdict", "anomaly")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    STATE_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    PROMOTION_FIELD_NUMBER: _ClassVar[int]
    RESULT_REF_FIELD_NUMBER: _ClassVar[int]
    WARRANT_REF_FIELD_NUMBER: _ClassVar[int]
    MOTE_DEF_HASH_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_SEQ_FIELD_NUMBER: _ClassVar[int]
    PARENTS_FIELD_NUMBER: _ClassVar[int]
    VERDICT_FIELD_NUMBER: _ClassVar[int]
    ANOMALY_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    state: MoteSnapshotState
    nd_class: _coordinator_pb2.NdClass
    promotion: PromotionState
    result_ref: bytes
    warrant_ref: bytes
    mote_def_hash: bytes
    committed_seq: int
    parents: _containers.RepeatedCompositeFieldContainer[_coordinator_pb2.ParentRef]
    verdict: bytes
    anomaly: MoteAnomaly
    def __init__(self, mote_id: _Optional[bytes] = ..., state: _Optional[_Union[MoteSnapshotState, str]] = ..., nd_class: _Optional[_Union[_coordinator_pb2.NdClass, str]] = ..., promotion: _Optional[_Union[PromotionState, str]] = ..., result_ref: _Optional[bytes] = ..., warrant_ref: _Optional[bytes] = ..., mote_def_hash: _Optional[bytes] = ..., committed_seq: _Optional[int] = ..., parents: _Optional[_Iterable[_Union[_coordinator_pb2.ParentRef, _Mapping]]] = ..., verdict: _Optional[bytes] = ..., anomaly: _Optional[_Union[MoteAnomaly, str]] = ...) -> None: ...

class GetContentRequest(_message.Message):
    __slots__ = ("content_ref", "instance_id")
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    content_ref: bytes
    instance_id: bytes
    def __init__(self, content_ref: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ...) -> None: ...

class ContentBlob(_message.Message):
    __slots__ = ("payload",)
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    payload: bytes
    def __init__(self, payload: _Optional[bytes] = ...) -> None: ...

class StreamEventsRequest(_message.Message):
    __slots__ = ("instance_id", "since_seq")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    SINCE_SEQ_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    since_seq: int
    def __init__(self, instance_id: _Optional[bytes] = ..., since_seq: _Optional[int] = ...) -> None: ...

class EventFrame(_message.Message):
    __slots__ = ("seq", "deltas", "next_seq", "journal_boundary")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    DELTAS_FIELD_NUMBER: _ClassVar[int]
    NEXT_SEQ_FIELD_NUMBER: _ClassVar[int]
    JOURNAL_BOUNDARY_FIELD_NUMBER: _ClassVar[int]
    seq: int
    deltas: _containers.RepeatedCompositeFieldContainer[EventDelta]
    next_seq: int
    journal_boundary: bool
    def __init__(self, seq: _Optional[int] = ..., deltas: _Optional[_Iterable[_Union[EventDelta, _Mapping]]] = ..., next_seq: _Optional[int] = ..., journal_boundary: bool = ...) -> None: ...

class EventDelta(_message.Message):
    __slots__ = ("seq", "committed", "failed", "repudiated", "effect_staged")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_FIELD_NUMBER: _ClassVar[int]
    FAILED_FIELD_NUMBER: _ClassVar[int]
    REPUDIATED_FIELD_NUMBER: _ClassVar[int]
    EFFECT_STAGED_FIELD_NUMBER: _ClassVar[int]
    seq: int
    committed: CommittedDelta
    failed: FailedDelta
    repudiated: RepudiatedDelta
    effect_staged: EffectStagedDelta
    def __init__(self, seq: _Optional[int] = ..., committed: _Optional[_Union[CommittedDelta, _Mapping]] = ..., failed: _Optional[_Union[FailedDelta, _Mapping]] = ..., repudiated: _Optional[_Union[RepudiatedDelta, _Mapping]] = ..., effect_staged: _Optional[_Union[EffectStagedDelta, _Mapping]] = ...) -> None: ...

class CommittedDelta(_message.Message):
    __slots__ = ("mote_id", "result_ref", "nd_class")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    RESULT_REF_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    result_ref: bytes
    nd_class: _coordinator_pb2.NdClass
    def __init__(self, mote_id: _Optional[bytes] = ..., result_ref: _Optional[bytes] = ..., nd_class: _Optional[_Union[_coordinator_pb2.NdClass, str]] = ...) -> None: ...

class FailedDelta(_message.Message):
    __slots__ = ("mote_id", "reason_class")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    REASON_CLASS_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    reason_class: int
    def __init__(self, mote_id: _Optional[bytes] = ..., reason_class: _Optional[int] = ...) -> None: ...

class RepudiatedDelta(_message.Message):
    __slots__ = ("target_mote_id", "target_committed_seq")
    TARGET_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    TARGET_COMMITTED_SEQ_FIELD_NUMBER: _ClassVar[int]
    target_mote_id: bytes
    target_committed_seq: int
    def __init__(self, target_mote_id: _Optional[bytes] = ..., target_committed_seq: _Optional[int] = ...) -> None: ...

class EffectStagedDelta(_message.Message):
    __slots__ = ("mote_id",)
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    def __init__(self, mote_id: _Optional[bytes] = ...) -> None: ...

class ListSignaturesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class SignatureSummary(_message.Message):
    __slots__ = ("signature_id", "name")
    SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    signature_id: bytes
    name: str
    def __init__(self, signature_id: _Optional[bytes] = ..., name: _Optional[str] = ...) -> None: ...

class ListSignaturesResponse(_message.Message):
    __slots__ = ("signatures",)
    SIGNATURES_FIELD_NUMBER: _ClassVar[int]
    signatures: _containers.RepeatedCompositeFieldContainer[SignatureSummary]
    def __init__(self, signatures: _Optional[_Iterable[_Union[SignatureSummary, _Mapping]]] = ...) -> None: ...

class GetSignatureRequest(_message.Message):
    __slots__ = ("signature_id",)
    SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    signature_id: bytes
    def __init__(self, signature_id: _Optional[bytes] = ...) -> None: ...

class GetSignatureResponse(_message.Message):
    __slots__ = ("signature_id", "manifest")
    SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    MANIFEST_FIELD_NUMBER: _ClassVar[int]
    signature_id: bytes
    manifest: bytes
    def __init__(self, signature_id: _Optional[bytes] = ..., manifest: _Optional[bytes] = ...) -> None: ...

class RegisterSignatureRequest(_message.Message):
    __slots__ = ("manifest",)
    MANIFEST_FIELD_NUMBER: _ClassVar[int]
    manifest: bytes
    def __init__(self, manifest: _Optional[bytes] = ...) -> None: ...

class RegisterSignatureResponse(_message.Message):
    __slots__ = ("signature_id",)
    SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    signature_id: bytes
    def __init__(self, signature_id: _Optional[bytes] = ...) -> None: ...

class ListRunsRequest(_message.Message):
    __slots__ = ("limit", "before_seq")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    BEFORE_SEQ_FIELD_NUMBER: _ClassVar[int]
    limit: int
    before_seq: int
    def __init__(self, limit: _Optional[int] = ..., before_seq: _Optional[int] = ...) -> None: ...

class RunSummary(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint", "registered_seq", "registered_unix_ms")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    REGISTERED_SEQ_FIELD_NUMBER: _ClassVar[int]
    REGISTERED_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    registered_seq: int
    registered_unix_ms: int
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ..., registered_seq: _Optional[int] = ..., registered_unix_ms: _Optional[int] = ...) -> None: ...

class ListRunsResponse(_message.Message):
    __slots__ = ("runs", "has_more")
    RUNS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    runs: _containers.RepeatedCompositeFieldContainer[RunSummary]
    has_more: bool
    def __init__(self, runs: _Optional[_Iterable[_Union[RunSummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class GetRunInputsRequest(_message.Message):
    __slots__ = ("instance_id",)
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    def __init__(self, instance_id: _Optional[bytes] = ...) -> None: ...

class GetRunInputsResponse(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint", "handle", "args")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    ARGS_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    handle: str
    args: bytes
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ..., handle: _Optional[str] = ..., args: _Optional[bytes] = ...) -> None: ...

class ListRecipesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class RecipeSummary(_message.Message):
    __slots__ = ("handle", "recipe_fingerprint", "description", "tags", "version")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    TAGS_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    handle: str
    recipe_fingerprint: bytes
    description: str
    tags: _containers.RepeatedScalarFieldContainer[str]
    version: str
    def __init__(self, handle: _Optional[str] = ..., recipe_fingerprint: _Optional[bytes] = ..., description: _Optional[str] = ..., tags: _Optional[_Iterable[str]] = ..., version: _Optional[str] = ...) -> None: ...

class ListRecipesResponse(_message.Message):
    __slots__ = ("recipes",)
    RECIPES_FIELD_NUMBER: _ClassVar[int]
    recipes: _containers.RepeatedCompositeFieldContainer[RecipeSummary]
    def __init__(self, recipes: _Optional[_Iterable[_Union[RecipeSummary, _Mapping]]] = ...) -> None: ...

class GetRecipeFormRequest(_message.Message):
    __slots__ = ("handle",)
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    def __init__(self, handle: _Optional[str] = ...) -> None: ...

class RecipeFormField(_message.Message):
    __slots__ = ("name", "type", "required", "max_len", "allowed")
    NAME_FIELD_NUMBER: _ClassVar[int]
    TYPE_FIELD_NUMBER: _ClassVar[int]
    REQUIRED_FIELD_NUMBER: _ClassVar[int]
    MAX_LEN_FIELD_NUMBER: _ClassVar[int]
    ALLOWED_FIELD_NUMBER: _ClassVar[int]
    name: str
    type: RecipeParamType
    required: bool
    max_len: int
    allowed: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, name: _Optional[str] = ..., type: _Optional[_Union[RecipeParamType, str]] = ..., required: bool = ..., max_len: _Optional[int] = ..., allowed: _Optional[_Iterable[str]] = ...) -> None: ...

class GetRecipeFormResponse(_message.Message):
    __slots__ = ("handle", "fields")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    FIELDS_FIELD_NUMBER: _ClassVar[int]
    handle: str
    fields: _containers.RepeatedCompositeFieldContainer[RecipeFormField]
    def __init__(self, handle: _Optional[str] = ..., fields: _Optional[_Iterable[_Union[RecipeFormField, _Mapping]]] = ...) -> None: ...

class SearchRecipesRequest(_message.Message):
    __slots__ = ("intent", "keywords", "limit")
    INTENT_FIELD_NUMBER: _ClassVar[int]
    KEYWORDS_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    intent: str
    keywords: _containers.RepeatedScalarFieldContainer[str]
    limit: int
    def __init__(self, intent: _Optional[str] = ..., keywords: _Optional[_Iterable[str]] = ..., limit: _Optional[int] = ...) -> None: ...

class ScoredRecipe(_message.Message):
    __slots__ = ("recipe", "score_bp")
    RECIPE_FIELD_NUMBER: _ClassVar[int]
    SCORE_BP_FIELD_NUMBER: _ClassVar[int]
    recipe: RecipeSummary
    score_bp: int
    def __init__(self, recipe: _Optional[_Union[RecipeSummary, _Mapping]] = ..., score_bp: _Optional[int] = ...) -> None: ...

class SearchRecipesResponse(_message.Message):
    __slots__ = ("ranked",)
    RANKED_FIELD_NUMBER: _ClassVar[int]
    ranked: _containers.RepeatedCompositeFieldContainer[ScoredRecipe]
    def __init__(self, ranked: _Optional[_Iterable[_Union[ScoredRecipe, _Mapping]]] = ...) -> None: ...

class ListTeamsRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class TeamSummary(_message.Message):
    __slots__ = ("team_id", "display_name", "owner", "member_count")
    TEAM_ID_FIELD_NUMBER: _ClassVar[int]
    DISPLAY_NAME_FIELD_NUMBER: _ClassVar[int]
    OWNER_FIELD_NUMBER: _ClassVar[int]
    MEMBER_COUNT_FIELD_NUMBER: _ClassVar[int]
    team_id: str
    display_name: str
    owner: str
    member_count: int
    def __init__(self, team_id: _Optional[str] = ..., display_name: _Optional[str] = ..., owner: _Optional[str] = ..., member_count: _Optional[int] = ...) -> None: ...

class ListTeamsResponse(_message.Message):
    __slots__ = ("teams",)
    TEAMS_FIELD_NUMBER: _ClassVar[int]
    teams: _containers.RepeatedCompositeFieldContainer[TeamSummary]
    def __init__(self, teams: _Optional[_Iterable[_Union[TeamSummary, _Mapping]]] = ...) -> None: ...

class ListTeamMembersRequest(_message.Message):
    __slots__ = ("team_id", "asset_ref")
    TEAM_ID_FIELD_NUMBER: _ClassVar[int]
    ASSET_REF_FIELD_NUMBER: _ClassVar[int]
    team_id: str
    asset_ref: str
    def __init__(self, team_id: _Optional[str] = ..., asset_ref: _Optional[str] = ...) -> None: ...

class WarrantView(_message.Message):
    __slots__ = ("executor_class", "model_route", "net_scope", "fs_scope", "max_calls", "cpu_milli", "wall_clock_ms")
    EXECUTOR_CLASS_FIELD_NUMBER: _ClassVar[int]
    MODEL_ROUTE_FIELD_NUMBER: _ClassVar[int]
    NET_SCOPE_FIELD_NUMBER: _ClassVar[int]
    FS_SCOPE_FIELD_NUMBER: _ClassVar[int]
    MAX_CALLS_FIELD_NUMBER: _ClassVar[int]
    CPU_MILLI_FIELD_NUMBER: _ClassVar[int]
    WALL_CLOCK_MS_FIELD_NUMBER: _ClassVar[int]
    executor_class: str
    model_route: str
    net_scope: str
    fs_scope: str
    max_calls: int
    cpu_milli: int
    wall_clock_ms: int
    def __init__(self, executor_class: _Optional[str] = ..., model_route: _Optional[str] = ..., net_scope: _Optional[str] = ..., fs_scope: _Optional[str] = ..., max_calls: _Optional[int] = ..., cpu_milli: _Optional[int] = ..., wall_clock_ms: _Optional[int] = ...) -> None: ...

class TeamMember(_message.Message):
    __slots__ = ("party", "role", "action_caps", "resolved_warrant")
    PARTY_FIELD_NUMBER: _ClassVar[int]
    ROLE_FIELD_NUMBER: _ClassVar[int]
    ACTION_CAPS_FIELD_NUMBER: _ClassVar[int]
    RESOLVED_WARRANT_FIELD_NUMBER: _ClassVar[int]
    party: str
    role: str
    action_caps: _containers.RepeatedScalarFieldContainer[str]
    resolved_warrant: WarrantView
    def __init__(self, party: _Optional[str] = ..., role: _Optional[str] = ..., action_caps: _Optional[_Iterable[str]] = ..., resolved_warrant: _Optional[_Union[WarrantView, _Mapping]] = ...) -> None: ...

class ListTeamMembersResponse(_message.Message):
    __slots__ = ("owner", "members")
    OWNER_FIELD_NUMBER: _ClassVar[int]
    MEMBERS_FIELD_NUMBER: _ClassVar[int]
    owner: str
    members: _containers.RepeatedCompositeFieldContainer[TeamMember]
    def __init__(self, owner: _Optional[str] = ..., members: _Optional[_Iterable[_Union[TeamMember, _Mapping]]] = ...) -> None: ...

class ListAssetGrantsRequest(_message.Message):
    __slots__ = ("asset_ref",)
    ASSET_REF_FIELD_NUMBER: _ClassVar[int]
    asset_ref: str
    def __init__(self, asset_ref: _Optional[str] = ...) -> None: ...

class GrantView(_message.Message):
    __slots__ = ("grantor", "grantee", "actions", "runtime_scope", "is_root", "revoked")
    GRANTOR_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_FIELD_NUMBER: _ClassVar[int]
    ACTIONS_FIELD_NUMBER: _ClassVar[int]
    RUNTIME_SCOPE_FIELD_NUMBER: _ClassVar[int]
    IS_ROOT_FIELD_NUMBER: _ClassVar[int]
    REVOKED_FIELD_NUMBER: _ClassVar[int]
    grantor: str
    grantee: str
    actions: _containers.RepeatedScalarFieldContainer[str]
    runtime_scope: str
    is_root: bool
    revoked: bool
    def __init__(self, grantor: _Optional[str] = ..., grantee: _Optional[str] = ..., actions: _Optional[_Iterable[str]] = ..., runtime_scope: _Optional[str] = ..., is_root: bool = ..., revoked: bool = ...) -> None: ...

class ListAssetGrantsResponse(_message.Message):
    __slots__ = ("owner", "grants")
    OWNER_FIELD_NUMBER: _ClassVar[int]
    GRANTS_FIELD_NUMBER: _ClassVar[int]
    owner: str
    grants: _containers.RepeatedCompositeFieldContainer[GrantView]
    def __init__(self, owner: _Optional[str] = ..., grants: _Optional[_Iterable[_Union[GrantView, _Mapping]]] = ...) -> None: ...

class ListDatasetsRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class DatasetSummary(_message.Message):
    __slots__ = ("dataset_id", "name", "doc_count", "dim", "created_ms", "chunked", "embed_model_fingerprint", "index_version", "chunk_count")
    DATASET_ID_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    DOC_COUNT_FIELD_NUMBER: _ClassVar[int]
    DIM_FIELD_NUMBER: _ClassVar[int]
    CREATED_MS_FIELD_NUMBER: _ClassVar[int]
    CHUNKED_FIELD_NUMBER: _ClassVar[int]
    EMBED_MODEL_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    INDEX_VERSION_FIELD_NUMBER: _ClassVar[int]
    CHUNK_COUNT_FIELD_NUMBER: _ClassVar[int]
    dataset_id: str
    name: str
    doc_count: int
    dim: int
    created_ms: int
    chunked: bool
    embed_model_fingerprint: str
    index_version: int
    chunk_count: int
    def __init__(self, dataset_id: _Optional[str] = ..., name: _Optional[str] = ..., doc_count: _Optional[int] = ..., dim: _Optional[int] = ..., created_ms: _Optional[int] = ..., chunked: bool = ..., embed_model_fingerprint: _Optional[str] = ..., index_version: _Optional[int] = ..., chunk_count: _Optional[int] = ...) -> None: ...

class ListDatasetsResponse(_message.Message):
    __slots__ = ("datasets",)
    DATASETS_FIELD_NUMBER: _ClassVar[int]
    datasets: _containers.RepeatedCompositeFieldContainer[DatasetSummary]
    def __init__(self, datasets: _Optional[_Iterable[_Union[DatasetSummary, _Mapping]]] = ...) -> None: ...

class IngestDocument(_message.Message):
    __slots__ = ("content", "embedding", "doc_id", "metadata")
    class MetadataEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: str
        def __init__(self, key: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    EMBEDDING_FIELD_NUMBER: _ClassVar[int]
    DOC_ID_FIELD_NUMBER: _ClassVar[int]
    METADATA_FIELD_NUMBER: _ClassVar[int]
    content: bytes
    embedding: _containers.RepeatedScalarFieldContainer[float]
    doc_id: str
    metadata: _containers.ScalarMap[str, str]
    def __init__(self, content: _Optional[bytes] = ..., embedding: _Optional[_Iterable[float]] = ..., doc_id: _Optional[str] = ..., metadata: _Optional[_Mapping[str, str]] = ...) -> None: ...

class IngestDocumentsRequest(_message.Message):
    __slots__ = ("dataset", "documents")
    DATASET_FIELD_NUMBER: _ClassVar[int]
    DOCUMENTS_FIELD_NUMBER: _ClassVar[int]
    dataset: str
    documents: _containers.RepeatedCompositeFieldContainer[IngestDocument]
    def __init__(self, dataset: _Optional[str] = ..., documents: _Optional[_Iterable[_Union[IngestDocument, _Mapping]]] = ...) -> None: ...

class IngestDocumentsResponse(_message.Message):
    __slots__ = ("dataset_id", "doc_count", "inserted", "dim")
    DATASET_ID_FIELD_NUMBER: _ClassVar[int]
    DOC_COUNT_FIELD_NUMBER: _ClassVar[int]
    INSERTED_FIELD_NUMBER: _ClassVar[int]
    DIM_FIELD_NUMBER: _ClassVar[int]
    dataset_id: str
    doc_count: int
    inserted: int
    dim: int
    def __init__(self, dataset_id: _Optional[str] = ..., doc_count: _Optional[int] = ..., inserted: _Optional[int] = ..., dim: _Optional[int] = ...) -> None: ...

class QueryDatasetRequest(_message.Message):
    __slots__ = ("dataset", "query_text", "query_embedding", "k", "retrieval_mode", "rerank")
    DATASET_FIELD_NUMBER: _ClassVar[int]
    QUERY_TEXT_FIELD_NUMBER: _ClassVar[int]
    QUERY_EMBEDDING_FIELD_NUMBER: _ClassVar[int]
    K_FIELD_NUMBER: _ClassVar[int]
    RETRIEVAL_MODE_FIELD_NUMBER: _ClassVar[int]
    RERANK_FIELD_NUMBER: _ClassVar[int]
    dataset: str
    query_text: str
    query_embedding: _containers.RepeatedScalarFieldContainer[float]
    k: int
    retrieval_mode: RetrievalMode
    rerank: bool
    def __init__(self, dataset: _Optional[str] = ..., query_text: _Optional[str] = ..., query_embedding: _Optional[_Iterable[float]] = ..., k: _Optional[int] = ..., retrieval_mode: _Optional[_Union[RetrievalMode, str]] = ..., rerank: bool = ...) -> None: ...

class DatasetHit(_message.Message):
    __slots__ = ("content_ref", "content", "score", "parent_ref", "chunk_index", "chunk_count")
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    SCORE_FIELD_NUMBER: _ClassVar[int]
    PARENT_REF_FIELD_NUMBER: _ClassVar[int]
    CHUNK_INDEX_FIELD_NUMBER: _ClassVar[int]
    CHUNK_COUNT_FIELD_NUMBER: _ClassVar[int]
    content_ref: bytes
    content: bytes
    score: float
    parent_ref: bytes
    chunk_index: int
    chunk_count: int
    def __init__(self, content_ref: _Optional[bytes] = ..., content: _Optional[bytes] = ..., score: _Optional[float] = ..., parent_ref: _Optional[bytes] = ..., chunk_index: _Optional[int] = ..., chunk_count: _Optional[int] = ...) -> None: ...

class QueryDatasetResponse(_message.Message):
    __slots__ = ("hits",)
    HITS_FIELD_NUMBER: _ClassVar[int]
    hits: _containers.RepeatedCompositeFieldContainer[DatasetHit]
    def __init__(self, hits: _Optional[_Iterable[_Union[DatasetHit, _Mapping]]] = ...) -> None: ...

class FuzzyDiscoveryRequest(_message.Message):
    __slots__ = ("dataset", "query_text", "query_embedding", "k", "retrieval_mode")
    DATASET_FIELD_NUMBER: _ClassVar[int]
    QUERY_TEXT_FIELD_NUMBER: _ClassVar[int]
    QUERY_EMBEDDING_FIELD_NUMBER: _ClassVar[int]
    K_FIELD_NUMBER: _ClassVar[int]
    RETRIEVAL_MODE_FIELD_NUMBER: _ClassVar[int]
    dataset: str
    query_text: str
    query_embedding: _containers.RepeatedScalarFieldContainer[float]
    k: int
    retrieval_mode: RetrievalMode
    def __init__(self, dataset: _Optional[str] = ..., query_text: _Optional[str] = ..., query_embedding: _Optional[_Iterable[float]] = ..., k: _Optional[int] = ..., retrieval_mode: _Optional[_Union[RetrievalMode, str]] = ...) -> None: ...

class FuzzyHit(_message.Message):
    __slots__ = ("content_ref", "score_bp", "parent_ref", "chunk_index")
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    SCORE_BP_FIELD_NUMBER: _ClassVar[int]
    PARENT_REF_FIELD_NUMBER: _ClassVar[int]
    CHUNK_INDEX_FIELD_NUMBER: _ClassVar[int]
    content_ref: bytes
    score_bp: int
    parent_ref: bytes
    chunk_index: int
    def __init__(self, content_ref: _Optional[bytes] = ..., score_bp: _Optional[int] = ..., parent_ref: _Optional[bytes] = ..., chunk_index: _Optional[int] = ...) -> None: ...

class FuzzyDiscoveryResponse(_message.Message):
    __slots__ = ("hits",)
    HITS_FIELD_NUMBER: _ClassVar[int]
    hits: _containers.RepeatedCompositeFieldContainer[FuzzyHit]
    def __init__(self, hits: _Optional[_Iterable[_Union[FuzzyHit, _Mapping]]] = ...) -> None: ...

class ListReplanRoundsRequest(_message.Message):
    __slots__ = ("limit",)
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    limit: int
    def __init__(self, limit: _Optional[int] = ...) -> None: ...

class ReplanRoundSummary(_message.Message):
    __slots__ = ("round", "shaper_mote_id", "model_id", "failed_step_ids", "escalated", "seq")
    ROUND_FIELD_NUMBER: _ClassVar[int]
    SHAPER_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    FAILED_STEP_IDS_FIELD_NUMBER: _ClassVar[int]
    ESCALATED_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    round: int
    shaper_mote_id: bytes
    model_id: str
    failed_step_ids: _containers.RepeatedScalarFieldContainer[bytes]
    escalated: bool
    seq: int
    def __init__(self, round: _Optional[int] = ..., shaper_mote_id: _Optional[bytes] = ..., model_id: _Optional[str] = ..., failed_step_ids: _Optional[_Iterable[bytes]] = ..., escalated: bool = ..., seq: _Optional[int] = ...) -> None: ...

class ListReplanRoundsResponse(_message.Message):
    __slots__ = ("rounds", "has_more")
    ROUNDS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    rounds: _containers.RepeatedCompositeFieldContainer[ReplanRoundSummary]
    has_more: bool
    def __init__(self, rounds: _Optional[_Iterable[_Union[ReplanRoundSummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class ListReactTurnsRequest(_message.Message):
    __slots__ = ("limit", "instance_id", "step_salt")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_SALT_FIELD_NUMBER: _ClassVar[int]
    limit: int
    instance_id: bytes
    step_salt: bytes
    def __init__(self, limit: _Optional[int] = ..., instance_id: _Optional[bytes] = ..., step_salt: _Optional[bytes] = ...) -> None: ...

class ReactTurnSummary(_message.Message):
    __slots__ = ("turn", "turn_mote_id", "instance_id", "model_id", "branch", "tool_id", "tool_version", "max_turns", "max_tool_calls", "seq", "rejection_reason", "step_salt", "call_index")
    TURN_FIELD_NUMBER: _ClassVar[int]
    TURN_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    BRANCH_FIELD_NUMBER: _ClassVar[int]
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    MAX_TURNS_FIELD_NUMBER: _ClassVar[int]
    MAX_TOOL_CALLS_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    REJECTION_REASON_FIELD_NUMBER: _ClassVar[int]
    STEP_SALT_FIELD_NUMBER: _ClassVar[int]
    CALL_INDEX_FIELD_NUMBER: _ClassVar[int]
    turn: int
    turn_mote_id: bytes
    instance_id: bytes
    model_id: str
    branch: str
    tool_id: str
    tool_version: str
    max_turns: int
    max_tool_calls: int
    seq: int
    rejection_reason: str
    step_salt: bytes
    call_index: int
    def __init__(self, turn: _Optional[int] = ..., turn_mote_id: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ..., model_id: _Optional[str] = ..., branch: _Optional[str] = ..., tool_id: _Optional[str] = ..., tool_version: _Optional[str] = ..., max_turns: _Optional[int] = ..., max_tool_calls: _Optional[int] = ..., seq: _Optional[int] = ..., rejection_reason: _Optional[str] = ..., step_salt: _Optional[bytes] = ..., call_index: _Optional[int] = ...) -> None: ...

class ListReactTurnsResponse(_message.Message):
    __slots__ = ("turns", "has_more")
    TURNS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    turns: _containers.RepeatedCompositeFieldContainer[ReactTurnSummary]
    has_more: bool
    def __init__(self, turns: _Optional[_Iterable[_Union[ReactTurnSummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class ListReRankTurnsRequest(_message.Message):
    __slots__ = ("limit", "instance_id")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    limit: int
    instance_id: bytes
    def __init__(self, limit: _Optional[int] = ..., instance_id: _Optional[bytes] = ...) -> None: ...

class ReRankTurnSummary(_message.Message):
    __slots__ = ("round", "rerank_mote_id", "instance_id", "model_id", "outcome", "candidate_count", "permutation", "seq")
    ROUND_FIELD_NUMBER: _ClassVar[int]
    RERANK_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    CANDIDATE_COUNT_FIELD_NUMBER: _ClassVar[int]
    PERMUTATION_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    round: int
    rerank_mote_id: bytes
    instance_id: bytes
    model_id: str
    outcome: str
    candidate_count: int
    permutation: _containers.RepeatedScalarFieldContainer[int]
    seq: int
    def __init__(self, round: _Optional[int] = ..., rerank_mote_id: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ..., model_id: _Optional[str] = ..., outcome: _Optional[str] = ..., candidate_count: _Optional[int] = ..., permutation: _Optional[_Iterable[int]] = ..., seq: _Optional[int] = ...) -> None: ...

class ListReRankTurnsResponse(_message.Message):
    __slots__ = ("turns", "has_more")
    TURNS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    turns: _containers.RepeatedCompositeFieldContainer[ReRankTurnSummary]
    has_more: bool
    def __init__(self, turns: _Optional[_Iterable[_Union[ReRankTurnSummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class StoreMemoryRequest(_message.Message):
    __slots__ = ("content", "embedding", "kind", "namespace")
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    EMBEDDING_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    content: bytes
    embedding: _containers.RepeatedScalarFieldContainer[float]
    kind: MemoryKind
    namespace: str
    def __init__(self, content: _Optional[bytes] = ..., embedding: _Optional[_Iterable[float]] = ..., kind: _Optional[_Union[MemoryKind, str]] = ..., namespace: _Optional[str] = ...) -> None: ...

class StoreMemoryResponse(_message.Message):
    __slots__ = ("memory_id", "inserted", "dim")
    MEMORY_ID_FIELD_NUMBER: _ClassVar[int]
    INSERTED_FIELD_NUMBER: _ClassVar[int]
    DIM_FIELD_NUMBER: _ClassVar[int]
    memory_id: bytes
    inserted: bool
    dim: int
    def __init__(self, memory_id: _Optional[bytes] = ..., inserted: bool = ..., dim: _Optional[int] = ...) -> None: ...

class ListMemoriesRequest(_message.Message):
    __slots__ = ("limit", "instance_id", "namespace", "include_tombstoned")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    INCLUDE_TOMBSTONED_FIELD_NUMBER: _ClassVar[int]
    limit: int
    instance_id: bytes
    namespace: str
    include_tombstoned: bool
    def __init__(self, limit: _Optional[int] = ..., instance_id: _Optional[bytes] = ..., namespace: _Optional[str] = ..., include_tombstoned: bool = ...) -> None: ...

class MemorySummary(_message.Message):
    __slots__ = ("memory_id", "content", "kind", "instance_id", "created_ms", "dim", "access_count", "last_accessed_ms", "tombstoned_ms")
    MEMORY_ID_FIELD_NUMBER: _ClassVar[int]
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    CREATED_MS_FIELD_NUMBER: _ClassVar[int]
    DIM_FIELD_NUMBER: _ClassVar[int]
    ACCESS_COUNT_FIELD_NUMBER: _ClassVar[int]
    LAST_ACCESSED_MS_FIELD_NUMBER: _ClassVar[int]
    TOMBSTONED_MS_FIELD_NUMBER: _ClassVar[int]
    memory_id: bytes
    content: bytes
    kind: str
    instance_id: bytes
    created_ms: int
    dim: int
    access_count: int
    last_accessed_ms: int
    tombstoned_ms: int
    def __init__(self, memory_id: _Optional[bytes] = ..., content: _Optional[bytes] = ..., kind: _Optional[str] = ..., instance_id: _Optional[bytes] = ..., created_ms: _Optional[int] = ..., dim: _Optional[int] = ..., access_count: _Optional[int] = ..., last_accessed_ms: _Optional[int] = ..., tombstoned_ms: _Optional[int] = ...) -> None: ...

class ListMemoriesResponse(_message.Message):
    __slots__ = ("memories", "has_more")
    MEMORIES_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    memories: _containers.RepeatedCompositeFieldContainer[MemorySummary]
    has_more: bool
    def __init__(self, memories: _Optional[_Iterable[_Union[MemorySummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class RecallMemoryRequest(_message.Message):
    __slots__ = ("query_text", "query_embedding", "k", "namespace")
    QUERY_TEXT_FIELD_NUMBER: _ClassVar[int]
    QUERY_EMBEDDING_FIELD_NUMBER: _ClassVar[int]
    K_FIELD_NUMBER: _ClassVar[int]
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    query_text: str
    query_embedding: _containers.RepeatedScalarFieldContainer[float]
    k: int
    namespace: str
    def __init__(self, query_text: _Optional[str] = ..., query_embedding: _Optional[_Iterable[float]] = ..., k: _Optional[int] = ..., namespace: _Optional[str] = ...) -> None: ...

class MemoryHit(_message.Message):
    __slots__ = ("memory_id", "content", "score")
    MEMORY_ID_FIELD_NUMBER: _ClassVar[int]
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    SCORE_FIELD_NUMBER: _ClassVar[int]
    memory_id: bytes
    content: bytes
    score: float
    def __init__(self, memory_id: _Optional[bytes] = ..., content: _Optional[bytes] = ..., score: _Optional[float] = ...) -> None: ...

class RecallMemoryResponse(_message.Message):
    __slots__ = ("hits",)
    HITS_FIELD_NUMBER: _ClassVar[int]
    hits: _containers.RepeatedCompositeFieldContainer[MemoryHit]
    def __init__(self, hits: _Optional[_Iterable[_Union[MemoryHit, _Mapping]]] = ...) -> None: ...

class ForgetMemoryRequest(_message.Message):
    __slots__ = ("memory_id", "namespace")
    MEMORY_ID_FIELD_NUMBER: _ClassVar[int]
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    memory_id: bytes
    namespace: str
    def __init__(self, memory_id: _Optional[bytes] = ..., namespace: _Optional[str] = ...) -> None: ...

class ForgetMemoryResponse(_message.Message):
    __slots__ = ("forgotten",)
    FORGOTTEN_FIELD_NUMBER: _ClassVar[int]
    forgotten: bool
    def __init__(self, forgotten: bool = ...) -> None: ...

class DecayMemoryRequest(_message.Message):
    __slots__ = ("namespace", "ttl_days", "min_access", "dry_run")
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    TTL_DAYS_FIELD_NUMBER: _ClassVar[int]
    MIN_ACCESS_FIELD_NUMBER: _ClassVar[int]
    DRY_RUN_FIELD_NUMBER: _ClassVar[int]
    namespace: str
    ttl_days: int
    min_access: int
    dry_run: bool
    def __init__(self, namespace: _Optional[str] = ..., ttl_days: _Optional[int] = ..., min_access: _Optional[int] = ..., dry_run: bool = ...) -> None: ...

class DecayCandidate(_message.Message):
    __slots__ = ("memory_id", "content", "kind", "created_ms", "access_count", "last_accessed_ms", "age_days")
    MEMORY_ID_FIELD_NUMBER: _ClassVar[int]
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    CREATED_MS_FIELD_NUMBER: _ClassVar[int]
    ACCESS_COUNT_FIELD_NUMBER: _ClassVar[int]
    LAST_ACCESSED_MS_FIELD_NUMBER: _ClassVar[int]
    AGE_DAYS_FIELD_NUMBER: _ClassVar[int]
    memory_id: bytes
    content: bytes
    kind: str
    created_ms: int
    access_count: int
    last_accessed_ms: int
    age_days: int
    def __init__(self, memory_id: _Optional[bytes] = ..., content: _Optional[bytes] = ..., kind: _Optional[str] = ..., created_ms: _Optional[int] = ..., access_count: _Optional[int] = ..., last_accessed_ms: _Optional[int] = ..., age_days: _Optional[int] = ...) -> None: ...

class DecayMemoryResponse(_message.Message):
    __slots__ = ("candidates", "would_evict", "evicted", "kept", "dry_run")
    CANDIDATES_FIELD_NUMBER: _ClassVar[int]
    WOULD_EVICT_FIELD_NUMBER: _ClassVar[int]
    EVICTED_FIELD_NUMBER: _ClassVar[int]
    KEPT_FIELD_NUMBER: _ClassVar[int]
    DRY_RUN_FIELD_NUMBER: _ClassVar[int]
    candidates: _containers.RepeatedCompositeFieldContainer[DecayCandidate]
    would_evict: int
    evicted: int
    kept: int
    dry_run: bool
    def __init__(self, candidates: _Optional[_Iterable[_Union[DecayCandidate, _Mapping]]] = ..., would_evict: _Optional[int] = ..., evicted: _Optional[int] = ..., kept: _Optional[int] = ..., dry_run: bool = ...) -> None: ...

class MemoryStatsRequest(_message.Message):
    __slots__ = ("namespace",)
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    namespace: str
    def __init__(self, namespace: _Optional[str] = ...) -> None: ...

class MemoryStatsResponse(_message.Message):
    __slots__ = ("total", "semantic", "episodic", "tombstoned", "dim", "embed_fingerprint", "oldest_ms", "newest_ms", "namespace")
    TOTAL_FIELD_NUMBER: _ClassVar[int]
    SEMANTIC_FIELD_NUMBER: _ClassVar[int]
    EPISODIC_FIELD_NUMBER: _ClassVar[int]
    TOMBSTONED_FIELD_NUMBER: _ClassVar[int]
    DIM_FIELD_NUMBER: _ClassVar[int]
    EMBED_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    OLDEST_MS_FIELD_NUMBER: _ClassVar[int]
    NEWEST_MS_FIELD_NUMBER: _ClassVar[int]
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    total: int
    semantic: int
    episodic: int
    tombstoned: int
    dim: int
    embed_fingerprint: str
    oldest_ms: int
    newest_ms: int
    namespace: str
    def __init__(self, total: _Optional[int] = ..., semantic: _Optional[int] = ..., episodic: _Optional[int] = ..., tombstoned: _Optional[int] = ..., dim: _Optional[int] = ..., embed_fingerprint: _Optional[str] = ..., oldest_ms: _Optional[int] = ..., newest_ms: _Optional[int] = ..., namespace: _Optional[str] = ...) -> None: ...

class RestoreMemoryRequest(_message.Message):
    __slots__ = ("memory_id", "namespace")
    MEMORY_ID_FIELD_NUMBER: _ClassVar[int]
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    memory_id: bytes
    namespace: str
    def __init__(self, memory_id: _Optional[bytes] = ..., namespace: _Optional[str] = ...) -> None: ...

class RestoreMemoryResponse(_message.Message):
    __slots__ = ("restored",)
    RESTORED_FIELD_NUMBER: _ClassVar[int]
    restored: bool
    def __init__(self, restored: bool = ...) -> None: ...

class ListCaptureRecordsRequest(_message.Message):
    __slots__ = ("limit", "instance_id")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    limit: int
    instance_id: bytes
    def __init__(self, limit: _Optional[int] = ..., instance_id: _Optional[bytes] = ...) -> None: ...

class CaptureRecordSummary(_message.Message):
    __slots__ = ("mote_id", "instance_id", "result_ref", "nd_class", "seq", "react_turn", "react_branch")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RESULT_REF_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    REACT_TURN_FIELD_NUMBER: _ClassVar[int]
    REACT_BRANCH_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    instance_id: bytes
    result_ref: bytes
    nd_class: str
    seq: int
    react_turn: int
    react_branch: str
    def __init__(self, mote_id: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ..., result_ref: _Optional[bytes] = ..., nd_class: _Optional[str] = ..., seq: _Optional[int] = ..., react_turn: _Optional[int] = ..., react_branch: _Optional[str] = ...) -> None: ...

class ListCaptureRecordsResponse(_message.Message):
    __slots__ = ("records", "has_more")
    RECORDS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    records: _containers.RepeatedCompositeFieldContainer[CaptureRecordSummary]
    has_more: bool
    def __init__(self, records: _Optional[_Iterable[_Union[CaptureRecordSummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class ListToolManifestsRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class KeywordSet(_message.Message):
    __slots__ = ("lang", "words")
    LANG_FIELD_NUMBER: _ClassVar[int]
    WORDS_FIELD_NUMBER: _ClassVar[int]
    lang: str
    words: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, lang: _Optional[str] = ..., words: _Optional[_Iterable[str]] = ...) -> None: ...

class ToolManifest(_message.Message):
    __slots__ = ("tool_id", "tool_version", "description", "keywords", "fingerprint_hash", "kind")
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    KEYWORDS_FIELD_NUMBER: _ClassVar[int]
    FINGERPRINT_HASH_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    tool_id: str
    tool_version: str
    description: str
    keywords: _containers.RepeatedCompositeFieldContainer[KeywordSet]
    fingerprint_hash: bytes
    kind: str
    def __init__(self, tool_id: _Optional[str] = ..., tool_version: _Optional[str] = ..., description: _Optional[str] = ..., keywords: _Optional[_Iterable[_Union[KeywordSet, _Mapping]]] = ..., fingerprint_hash: _Optional[bytes] = ..., kind: _Optional[str] = ...) -> None: ...

class ListToolManifestsResponse(_message.Message):
    __slots__ = ("manifests",)
    MANIFESTS_FIELD_NUMBER: _ClassVar[int]
    manifests: _containers.RepeatedCompositeFieldContainer[ToolManifest]
    def __init__(self, manifests: _Optional[_Iterable[_Union[ToolManifest, _Mapping]]] = ...) -> None: ...

class BundleToolSpec(_message.Message):
    __slots__ = ("tool_id", "tool_version", "description", "keywords")
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    KEYWORDS_FIELD_NUMBER: _ClassVar[int]
    tool_id: str
    tool_version: str
    description: str
    keywords: _containers.RepeatedCompositeFieldContainer[KeywordSet]
    def __init__(self, tool_id: _Optional[str] = ..., tool_version: _Optional[str] = ..., description: _Optional[str] = ..., keywords: _Optional[_Iterable[_Union[KeywordSet, _Mapping]]] = ...) -> None: ...

class ScoreTaskBundleRequest(_message.Message):
    __slots__ = ("intent", "language_tags", "tool_sequence", "tolerance_threshold_bp")
    INTENT_FIELD_NUMBER: _ClassVar[int]
    LANGUAGE_TAGS_FIELD_NUMBER: _ClassVar[int]
    TOOL_SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    TOLERANCE_THRESHOLD_BP_FIELD_NUMBER: _ClassVar[int]
    intent: str
    language_tags: _containers.RepeatedScalarFieldContainer[str]
    tool_sequence: _containers.RepeatedCompositeFieldContainer[BundleToolSpec]
    tolerance_threshold_bp: int
    def __init__(self, intent: _Optional[str] = ..., language_tags: _Optional[_Iterable[str]] = ..., tool_sequence: _Optional[_Iterable[_Union[BundleToolSpec, _Mapping]]] = ..., tolerance_threshold_bp: _Optional[int] = ...) -> None: ...

class ManifestScore(_message.Message):
    __slots__ = ("tool_id", "tool_version", "score_bp", "fingerprint_hash")
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    SCORE_BP_FIELD_NUMBER: _ClassVar[int]
    FINGERPRINT_HASH_FIELD_NUMBER: _ClassVar[int]
    tool_id: str
    tool_version: str
    score_bp: int
    fingerprint_hash: bytes
    def __init__(self, tool_id: _Optional[str] = ..., tool_version: _Optional[str] = ..., score_bp: _Optional[int] = ..., fingerprint_hash: _Optional[bytes] = ...) -> None: ...

class ScoreTaskBundleResponse(_message.Message):
    __slots__ = ("bundle_fingerprint", "ranked", "verdict", "verdict_detail")
    BUNDLE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    RANKED_FIELD_NUMBER: _ClassVar[int]
    VERDICT_FIELD_NUMBER: _ClassVar[int]
    VERDICT_DETAIL_FIELD_NUMBER: _ClassVar[int]
    bundle_fingerprint: bytes
    ranked: _containers.RepeatedCompositeFieldContainer[ManifestScore]
    verdict: LowerVerdict
    verdict_detail: str
    def __init__(self, bundle_fingerprint: _Optional[bytes] = ..., ranked: _Optional[_Iterable[_Union[ManifestScore, _Mapping]]] = ..., verdict: _Optional[_Union[LowerVerdict, str]] = ..., verdict_detail: _Optional[str] = ...) -> None: ...

class WorkflowStep(_message.Message):
    __slots__ = ("kind", "model_id", "prompt", "body_signature_id", "tool_contract", "params")
    class ToolContractEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: str
        def __init__(self, key: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...
    class ParamsEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: bytes
        def __init__(self, key: _Optional[str] = ..., value: _Optional[bytes] = ...) -> None: ...
    KIND_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    PROMPT_FIELD_NUMBER: _ClassVar[int]
    BODY_SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_CONTRACT_FIELD_NUMBER: _ClassVar[int]
    PARAMS_FIELD_NUMBER: _ClassVar[int]
    kind: WorkflowStepKind
    model_id: str
    prompt: str
    body_signature_id: bytes
    tool_contract: _containers.ScalarMap[str, str]
    params: _containers.ScalarMap[str, bytes]
    def __init__(self, kind: _Optional[_Union[WorkflowStepKind, str]] = ..., model_id: _Optional[str] = ..., prompt: _Optional[str] = ..., body_signature_id: _Optional[bytes] = ..., tool_contract: _Optional[_Mapping[str, str]] = ..., params: _Optional[_Mapping[str, bytes]] = ...) -> None: ...

class WorkflowEdge(_message.Message):
    __slots__ = ("parent", "child", "edge_kind", "non_cascade")
    PARENT_FIELD_NUMBER: _ClassVar[int]
    CHILD_FIELD_NUMBER: _ClassVar[int]
    EDGE_KIND_FIELD_NUMBER: _ClassVar[int]
    NON_CASCADE_FIELD_NUMBER: _ClassVar[int]
    parent: int
    child: int
    edge_kind: _coordinator_pb2.EdgeKind
    non_cascade: bool
    def __init__(self, parent: _Optional[int] = ..., child: _Optional[int] = ..., edge_kind: _Optional[_Union[_coordinator_pb2.EdgeKind, str]] = ..., non_cascade: bool = ...) -> None: ...

class SubmitWorkflowRequest(_message.Message):
    __slots__ = ("seed", "steps", "edges", "execution_mode", "context_bundles")
    SEED_FIELD_NUMBER: _ClassVar[int]
    STEPS_FIELD_NUMBER: _ClassVar[int]
    EDGES_FIELD_NUMBER: _ClassVar[int]
    EXECUTION_MODE_FIELD_NUMBER: _ClassVar[int]
    CONTEXT_BUNDLES_FIELD_NUMBER: _ClassVar[int]
    seed: int
    steps: _containers.RepeatedCompositeFieldContainer[WorkflowStep]
    edges: _containers.RepeatedCompositeFieldContainer[WorkflowEdge]
    execution_mode: WorkflowExecutionMode
    context_bundles: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, seed: _Optional[int] = ..., steps: _Optional[_Iterable[_Union[WorkflowStep, _Mapping]]] = ..., edges: _Optional[_Iterable[_Union[WorkflowEdge, _Mapping]]] = ..., execution_mode: _Optional[_Union[WorkflowExecutionMode, str]] = ..., context_bundles: _Optional[_Iterable[str]] = ...) -> None: ...

class PutContentRequest(_message.Message):
    __slots__ = ("payload", "media_type", "filename")
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    MEDIA_TYPE_FIELD_NUMBER: _ClassVar[int]
    FILENAME_FIELD_NUMBER: _ClassVar[int]
    payload: bytes
    media_type: str
    filename: str
    def __init__(self, payload: _Optional[bytes] = ..., media_type: _Optional[str] = ..., filename: _Optional[str] = ...) -> None: ...

class PutContentResponse(_message.Message):
    __slots__ = ("content_ref", "size", "deduplicated")
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    SIZE_FIELD_NUMBER: _ClassVar[int]
    DEDUPLICATED_FIELD_NUMBER: _ClassVar[int]
    content_ref: bytes
    size: int
    deduplicated: bool
    def __init__(self, content_ref: _Optional[bytes] = ..., size: _Optional[int] = ..., deduplicated: bool = ...) -> None: ...

class GetContentBatchRequest(_message.Message):
    __slots__ = ("instance_id", "content_refs", "max_bytes_per_item")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    CONTENT_REFS_FIELD_NUMBER: _ClassVar[int]
    MAX_BYTES_PER_ITEM_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    content_refs: _containers.RepeatedScalarFieldContainer[bytes]
    max_bytes_per_item: int
    def __init__(self, instance_id: _Optional[bytes] = ..., content_refs: _Optional[_Iterable[bytes]] = ..., max_bytes_per_item: _Optional[int] = ...) -> None: ...

class ContentBatchItem(_message.Message):
    __slots__ = ("content_ref", "payload", "truncated", "full_size")
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    TRUNCATED_FIELD_NUMBER: _ClassVar[int]
    FULL_SIZE_FIELD_NUMBER: _ClassVar[int]
    content_ref: bytes
    payload: bytes
    truncated: bool
    full_size: int
    def __init__(self, content_ref: _Optional[bytes] = ..., payload: _Optional[bytes] = ..., truncated: bool = ..., full_size: _Optional[int] = ...) -> None: ...

class GetContentBatchResponse(_message.Message):
    __slots__ = ("items",)
    ITEMS_FIELD_NUMBER: _ClassVar[int]
    items: _containers.RepeatedCompositeFieldContainer[ContentBatchItem]
    def __init__(self, items: _Optional[_Iterable[_Union[ContentBatchItem, _Mapping]]] = ...) -> None: ...

class ListModelsRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ModelSummary(_message.Message):
    __slots__ = ("model_id", "modalities", "description", "serving", "context_len", "loaded", "chat_handle", "engine", "can_embed", "source", "active", "chat_rag_handle", "embed_is_decoder")
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    MODALITIES_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    SERVING_FIELD_NUMBER: _ClassVar[int]
    CONTEXT_LEN_FIELD_NUMBER: _ClassVar[int]
    LOADED_FIELD_NUMBER: _ClassVar[int]
    CHAT_HANDLE_FIELD_NUMBER: _ClassVar[int]
    ENGINE_FIELD_NUMBER: _ClassVar[int]
    CAN_EMBED_FIELD_NUMBER: _ClassVar[int]
    SOURCE_FIELD_NUMBER: _ClassVar[int]
    ACTIVE_FIELD_NUMBER: _ClassVar[int]
    CHAT_RAG_HANDLE_FIELD_NUMBER: _ClassVar[int]
    EMBED_IS_DECODER_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    modalities: _containers.RepeatedScalarFieldContainer[str]
    description: str
    serving: bool
    context_len: int
    loaded: bool
    chat_handle: str
    engine: str
    can_embed: bool
    source: str
    active: bool
    chat_rag_handle: str
    embed_is_decoder: bool
    def __init__(self, model_id: _Optional[str] = ..., modalities: _Optional[_Iterable[str]] = ..., description: _Optional[str] = ..., serving: bool = ..., context_len: _Optional[int] = ..., loaded: bool = ..., chat_handle: _Optional[str] = ..., engine: _Optional[str] = ..., can_embed: bool = ..., source: _Optional[str] = ..., active: bool = ..., chat_rag_handle: _Optional[str] = ..., embed_is_decoder: bool = ...) -> None: ...

class ListModelsResponse(_message.Message):
    __slots__ = ("models",)
    MODELS_FIELD_NUMBER: _ClassVar[int]
    models: _containers.RepeatedCompositeFieldContainer[ModelSummary]
    def __init__(self, models: _Optional[_Iterable[_Union[ModelSummary, _Mapping]]] = ...) -> None: ...

class LoadModelRequest(_message.Message):
    __slots__ = ("model_id",)
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    def __init__(self, model_id: _Optional[str] = ...) -> None: ...

class LoadModelResponse(_message.Message):
    __slots__ = ("model_id", "loaded", "was_resident")
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    LOADED_FIELD_NUMBER: _ClassVar[int]
    WAS_RESIDENT_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    loaded: bool
    was_resident: bool
    def __init__(self, model_id: _Optional[str] = ..., loaded: bool = ..., was_resident: bool = ...) -> None: ...

class OffloadModelRequest(_message.Message):
    __slots__ = ("model_id",)
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    def __init__(self, model_id: _Optional[str] = ...) -> None: ...

class OffloadModelResponse(_message.Message):
    __slots__ = ("model_id", "loaded", "was_resident")
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    LOADED_FIELD_NUMBER: _ClassVar[int]
    WAS_RESIDENT_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    loaded: bool
    was_resident: bool
    def __init__(self, model_id: _Optional[str] = ..., loaded: bool = ..., was_resident: bool = ...) -> None: ...

class PullModelRequest(_message.Message):
    __slots__ = ("ollama_tag", "url", "sha256", "model_id")
    OLLAMA_TAG_FIELD_NUMBER: _ClassVar[int]
    URL_FIELD_NUMBER: _ClassVar[int]
    SHA256_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    ollama_tag: str
    url: str
    sha256: str
    model_id: str
    def __init__(self, ollama_tag: _Optional[str] = ..., url: _Optional[str] = ..., sha256: _Optional[str] = ..., model_id: _Optional[str] = ...) -> None: ...

class PullModelResponse(_message.Message):
    __slots__ = ("model_id", "accepted", "detail")
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    ACCEPTED_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    accepted: bool
    detail: str
    def __init__(self, model_id: _Optional[str] = ..., accepted: bool = ..., detail: _Optional[str] = ...) -> None: ...

class GetPullStatusRequest(_message.Message):
    __slots__ = ("model_id",)
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    def __init__(self, model_id: _Optional[str] = ...) -> None: ...

class GetPullStatusResponse(_message.Message):
    __slots__ = ("phase", "bytes_downloaded", "bytes_total", "detail")
    class Phase(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
        __slots__ = ()
        PHASE_UNSPECIFIED: _ClassVar[GetPullStatusResponse.Phase]
        RESOLVING: _ClassVar[GetPullStatusResponse.Phase]
        DOWNLOADING: _ClassVar[GetPullStatusResponse.Phase]
        VERIFYING: _ClassVar[GetPullStatusResponse.Phase]
        REGISTERING: _ClassVar[GetPullStatusResponse.Phase]
        DONE: _ClassVar[GetPullStatusResponse.Phase]
        FAILED: _ClassVar[GetPullStatusResponse.Phase]
    PHASE_UNSPECIFIED: GetPullStatusResponse.Phase
    RESOLVING: GetPullStatusResponse.Phase
    DOWNLOADING: GetPullStatusResponse.Phase
    VERIFYING: GetPullStatusResponse.Phase
    REGISTERING: GetPullStatusResponse.Phase
    DONE: GetPullStatusResponse.Phase
    FAILED: GetPullStatusResponse.Phase
    PHASE_FIELD_NUMBER: _ClassVar[int]
    BYTES_DOWNLOADED_FIELD_NUMBER: _ClassVar[int]
    BYTES_TOTAL_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    phase: GetPullStatusResponse.Phase
    bytes_downloaded: int
    bytes_total: int
    detail: str
    def __init__(self, phase: _Optional[_Union[GetPullStatusResponse.Phase, str]] = ..., bytes_downloaded: _Optional[int] = ..., bytes_total: _Optional[int] = ..., detail: _Optional[str] = ...) -> None: ...

class SetActiveModelRequest(_message.Message):
    __slots__ = ("model_id",)
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    def __init__(self, model_id: _Optional[str] = ...) -> None: ...

class SetActiveModelResponse(_message.Message):
    __slots__ = ("active_model_id",)
    ACTIVE_MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    active_model_id: str
    def __init__(self, active_model_id: _Optional[str] = ...) -> None: ...

class AppSummary(_message.Message):
    __slots__ = ("handle", "app_ref", "name", "version", "description", "tags", "step_count", "locked")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    APP_REF_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    TAGS_FIELD_NUMBER: _ClassVar[int]
    STEP_COUNT_FIELD_NUMBER: _ClassVar[int]
    LOCKED_FIELD_NUMBER: _ClassVar[int]
    handle: str
    app_ref: bytes
    name: str
    version: str
    description: str
    tags: _containers.RepeatedScalarFieldContainer[str]
    step_count: int
    locked: bool
    def __init__(self, handle: _Optional[str] = ..., app_ref: _Optional[bytes] = ..., name: _Optional[str] = ..., version: _Optional[str] = ..., description: _Optional[str] = ..., tags: _Optional[_Iterable[str]] = ..., step_count: _Optional[int] = ..., locked: bool = ...) -> None: ...

class SaveAppRequest(_message.Message):
    __slots__ = ("handle", "envelope_json")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    ENVELOPE_JSON_FIELD_NUMBER: _ClassVar[int]
    handle: str
    envelope_json: bytes
    def __init__(self, handle: _Optional[str] = ..., envelope_json: _Optional[bytes] = ...) -> None: ...

class SaveAppResponse(_message.Message):
    __slots__ = ("app_ref", "handle", "deduplicated")
    APP_REF_FIELD_NUMBER: _ClassVar[int]
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    DEDUPLICATED_FIELD_NUMBER: _ClassVar[int]
    app_ref: bytes
    handle: str
    deduplicated: bool
    def __init__(self, app_ref: _Optional[bytes] = ..., handle: _Optional[str] = ..., deduplicated: bool = ...) -> None: ...

class ListAppsRequest(_message.Message):
    __slots__ = ("limit", "after_handle")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    AFTER_HANDLE_FIELD_NUMBER: _ClassVar[int]
    limit: int
    after_handle: str
    def __init__(self, limit: _Optional[int] = ..., after_handle: _Optional[str] = ...) -> None: ...

class ListAppsResponse(_message.Message):
    __slots__ = ("apps", "has_more")
    APPS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    apps: _containers.RepeatedCompositeFieldContainer[AppSummary]
    has_more: bool
    def __init__(self, apps: _Optional[_Iterable[_Union[AppSummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class GetAppRequest(_message.Message):
    __slots__ = ("handle",)
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    def __init__(self, handle: _Optional[str] = ...) -> None: ...

class GetAppResponse(_message.Message):
    __slots__ = ("found", "envelope_json", "summary")
    FOUND_FIELD_NUMBER: _ClassVar[int]
    ENVELOPE_JSON_FIELD_NUMBER: _ClassVar[int]
    SUMMARY_FIELD_NUMBER: _ClassVar[int]
    found: bool
    envelope_json: bytes
    summary: AppSummary
    def __init__(self, found: bool = ..., envelope_json: _Optional[bytes] = ..., summary: _Optional[_Union[AppSummary, _Mapping]] = ...) -> None: ...

class ScaffoldAppRequest(_message.Message):
    __slots__ = ("handle", "branch_handle", "instruction")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    BRANCH_HANDLE_FIELD_NUMBER: _ClassVar[int]
    INSTRUCTION_FIELD_NUMBER: _ClassVar[int]
    handle: str
    branch_handle: str
    instruction: str
    def __init__(self, handle: _Optional[str] = ..., branch_handle: _Optional[str] = ..., instruction: _Optional[str] = ...) -> None: ...

class ScaffoldAppResponse(_message.Message):
    __slots__ = ("instance_id", "branch_handle", "resumed")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    BRANCH_HANDLE_FIELD_NUMBER: _ClassVar[int]
    RESUMED_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    branch_handle: str
    resumed: bool
    def __init__(self, instance_id: _Optional[bytes] = ..., branch_handle: _Optional[str] = ..., resumed: bool = ...) -> None: ...

class GetScaffoldStatusRequest(_message.Message):
    __slots__ = ("branch_handle",)
    BRANCH_HANDLE_FIELD_NUMBER: _ClassVar[int]
    branch_handle: str
    def __init__(self, branch_handle: _Optional[str] = ...) -> None: ...

class GetScaffoldStatusResponse(_message.Message):
    __slots__ = ("phase", "files_done", "files_pending", "detail")
    class Phase(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
        __slots__ = ()
        PHASE_UNSPECIFIED: _ClassVar[GetScaffoldStatusResponse.Phase]
        PLANNING: _ClassVar[GetScaffoldStatusResponse.Phase]
        WRITING: _ClassVar[GetScaffoldStatusResponse.Phase]
        DONE: _ClassVar[GetScaffoldStatusResponse.Phase]
        FAILED: _ClassVar[GetScaffoldStatusResponse.Phase]
    PHASE_UNSPECIFIED: GetScaffoldStatusResponse.Phase
    PLANNING: GetScaffoldStatusResponse.Phase
    WRITING: GetScaffoldStatusResponse.Phase
    DONE: GetScaffoldStatusResponse.Phase
    FAILED: GetScaffoldStatusResponse.Phase
    PHASE_FIELD_NUMBER: _ClassVar[int]
    FILES_DONE_FIELD_NUMBER: _ClassVar[int]
    FILES_PENDING_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    phase: GetScaffoldStatusResponse.Phase
    files_done: _containers.RepeatedScalarFieldContainer[str]
    files_pending: _containers.RepeatedScalarFieldContainer[str]
    detail: str
    def __init__(self, phase: _Optional[_Union[GetScaffoldStatusResponse.Phase, str]] = ..., files_done: _Optional[_Iterable[str]] = ..., files_pending: _Optional[_Iterable[str]] = ..., detail: _Optional[str] = ...) -> None: ...

class LockAppRequest(_message.Message):
    __slots__ = ("branch_handle",)
    BRANCH_HANDLE_FIELD_NUMBER: _ClassVar[int]
    branch_handle: str
    def __init__(self, branch_handle: _Optional[str] = ...) -> None: ...

class LockAppResponse(_message.Message):
    __slots__ = ("locked",)
    LOCKED_FIELD_NUMBER: _ClassVar[int]
    locked: bool
    def __init__(self, locked: bool = ...) -> None: ...

class UnlockAppRequest(_message.Message):
    __slots__ = ("branch_handle",)
    BRANCH_HANDLE_FIELD_NUMBER: _ClassVar[int]
    branch_handle: str
    def __init__(self, branch_handle: _Optional[str] = ...) -> None: ...

class UnlockAppResponse(_message.Message):
    __slots__ = ("unlocked",)
    UNLOCKED_FIELD_NUMBER: _ClassVar[int]
    unlocked: bool
    def __init__(self, unlocked: bool = ...) -> None: ...

class GetMoteDetailRequest(_message.Message):
    __slots__ = ("instance_id", "mote_id")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    mote_id: bytes
    def __init__(self, instance_id: _Optional[bytes] = ..., mote_id: _Optional[bytes] = ...) -> None: ...

class MoteConfigEntry(_message.Message):
    __slots__ = ("key", "value", "truncated", "full_len")
    KEY_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    TRUNCATED_FIELD_NUMBER: _ClassVar[int]
    FULL_LEN_FIELD_NUMBER: _ClassVar[int]
    key: str
    value: bytes
    truncated: bool
    full_len: int
    def __init__(self, key: _Optional[str] = ..., value: _Optional[bytes] = ..., truncated: bool = ..., full_len: _Optional[int] = ...) -> None: ...

class MoteDetail(_message.Message):
    __slots__ = ("mote_id", "mote_def_hash", "def_found", "step_kind", "model_id", "prompt", "prompt_truncated", "config_subset", "tool_contract", "logic_ref", "nd_class", "effect_pattern", "critic_for", "is_topology_shaper", "schema_version")
    class ToolContractEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: str
        def __init__(self, key: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    MOTE_DEF_HASH_FIELD_NUMBER: _ClassVar[int]
    DEF_FOUND_FIELD_NUMBER: _ClassVar[int]
    STEP_KIND_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    PROMPT_FIELD_NUMBER: _ClassVar[int]
    PROMPT_TRUNCATED_FIELD_NUMBER: _ClassVar[int]
    CONFIG_SUBSET_FIELD_NUMBER: _ClassVar[int]
    TOOL_CONTRACT_FIELD_NUMBER: _ClassVar[int]
    LOGIC_REF_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    EFFECT_PATTERN_FIELD_NUMBER: _ClassVar[int]
    CRITIC_FOR_FIELD_NUMBER: _ClassVar[int]
    IS_TOPOLOGY_SHAPER_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_VERSION_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    mote_def_hash: bytes
    def_found: bool
    step_kind: str
    model_id: str
    prompt: str
    prompt_truncated: bool
    config_subset: _containers.RepeatedCompositeFieldContainer[MoteConfigEntry]
    tool_contract: _containers.ScalarMap[str, str]
    logic_ref: bytes
    nd_class: _coordinator_pb2.NdClass
    effect_pattern: _coordinator_pb2.EffectPattern
    critic_for: bytes
    is_topology_shaper: bool
    schema_version: int
    def __init__(self, mote_id: _Optional[bytes] = ..., mote_def_hash: _Optional[bytes] = ..., def_found: bool = ..., step_kind: _Optional[str] = ..., model_id: _Optional[str] = ..., prompt: _Optional[str] = ..., prompt_truncated: bool = ..., config_subset: _Optional[_Iterable[_Union[MoteConfigEntry, _Mapping]]] = ..., tool_contract: _Optional[_Mapping[str, str]] = ..., logic_ref: _Optional[bytes] = ..., nd_class: _Optional[_Union[_coordinator_pb2.NdClass, str]] = ..., effect_pattern: _Optional[_Union[_coordinator_pb2.EffectPattern, str]] = ..., critic_for: _Optional[bytes] = ..., is_topology_shaper: bool = ..., schema_version: _Optional[int] = ...) -> None: ...

class StreamAllEventsRequest(_message.Message):
    __slots__ = ("since_seq",)
    SINCE_SEQ_FIELD_NUMBER: _ClassVar[int]
    since_seq: int
    def __init__(self, since_seq: _Optional[int] = ...) -> None: ...

class RunRegisteredDelta(_message.Message):
    __slots__ = ("recipe_fingerprint", "registered_unix_ms")
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    REGISTERED_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    recipe_fingerprint: bytes
    registered_unix_ms: int
    def __init__(self, recipe_fingerprint: _Optional[bytes] = ..., registered_unix_ms: _Optional[int] = ...) -> None: ...

class GlobalEventDelta(_message.Message):
    __slots__ = ("seq", "instance_id", "committed", "failed", "repudiated", "effect_staged", "run_registered")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_FIELD_NUMBER: _ClassVar[int]
    FAILED_FIELD_NUMBER: _ClassVar[int]
    REPUDIATED_FIELD_NUMBER: _ClassVar[int]
    EFFECT_STAGED_FIELD_NUMBER: _ClassVar[int]
    RUN_REGISTERED_FIELD_NUMBER: _ClassVar[int]
    seq: int
    instance_id: bytes
    committed: CommittedDelta
    failed: FailedDelta
    repudiated: RepudiatedDelta
    effect_staged: EffectStagedDelta
    run_registered: RunRegisteredDelta
    def __init__(self, seq: _Optional[int] = ..., instance_id: _Optional[bytes] = ..., committed: _Optional[_Union[CommittedDelta, _Mapping]] = ..., failed: _Optional[_Union[FailedDelta, _Mapping]] = ..., repudiated: _Optional[_Union[RepudiatedDelta, _Mapping]] = ..., effect_staged: _Optional[_Union[EffectStagedDelta, _Mapping]] = ..., run_registered: _Optional[_Union[RunRegisteredDelta, _Mapping]] = ...) -> None: ...

class GlobalEventFrame(_message.Message):
    __slots__ = ("seq", "deltas", "next_seq", "journal_boundary")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    DELTAS_FIELD_NUMBER: _ClassVar[int]
    NEXT_SEQ_FIELD_NUMBER: _ClassVar[int]
    JOURNAL_BOUNDARY_FIELD_NUMBER: _ClassVar[int]
    seq: int
    deltas: _containers.RepeatedCompositeFieldContainer[GlobalEventDelta]
    next_seq: int
    journal_boundary: bool
    def __init__(self, seq: _Optional[int] = ..., deltas: _Optional[_Iterable[_Union[GlobalEventDelta, _Mapping]]] = ..., next_seq: _Optional[int] = ..., journal_boundary: bool = ...) -> None: ...

class ListMoteTelemetryRequest(_message.Message):
    __slots__ = ("limit", "instance_id", "mote_id", "before_seq")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    BEFORE_SEQ_FIELD_NUMBER: _ClassVar[int]
    limit: int
    instance_id: bytes
    mote_id: bytes
    before_seq: int
    def __init__(self, limit: _Optional[int] = ..., instance_id: _Optional[bytes] = ..., mote_id: _Optional[bytes] = ..., before_seq: _Optional[int] = ...) -> None: ...

class MoteTelemetryRow(_message.Message):
    __slots__ = ("mote_id", "instance_id", "wall_clock_ms", "input_tokens", "output_tokens", "model_id", "tool_id", "started_unix_ms", "seq")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    WALL_CLOCK_MS_FIELD_NUMBER: _ClassVar[int]
    INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    STARTED_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    instance_id: bytes
    wall_clock_ms: int
    input_tokens: int
    output_tokens: int
    model_id: str
    tool_id: str
    started_unix_ms: int
    seq: int
    def __init__(self, mote_id: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ..., wall_clock_ms: _Optional[int] = ..., input_tokens: _Optional[int] = ..., output_tokens: _Optional[int] = ..., model_id: _Optional[str] = ..., tool_id: _Optional[str] = ..., started_unix_ms: _Optional[int] = ..., seq: _Optional[int] = ...) -> None: ...

class ListMoteTelemetryResponse(_message.Message):
    __slots__ = ("rows", "has_more")
    ROWS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    rows: _containers.RepeatedCompositeFieldContainer[MoteTelemetryRow]
    has_more: bool
    def __init__(self, rows: _Optional[_Iterable[_Union[MoteTelemetryRow, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class ListTelemetrySummaryRequest(_message.Message):
    __slots__ = ("instance_id",)
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    def __init__(self, instance_id: _Optional[bytes] = ...) -> None: ...

class ModelTokenRollup(_message.Message):
    __slots__ = ("model_id", "count", "total_output_tokens", "total_wall_clock_ms")
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    COUNT_FIELD_NUMBER: _ClassVar[int]
    TOTAL_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    TOTAL_WALL_CLOCK_MS_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    count: int
    total_output_tokens: int
    total_wall_clock_ms: int
    def __init__(self, model_id: _Optional[str] = ..., count: _Optional[int] = ..., total_output_tokens: _Optional[int] = ..., total_wall_clock_ms: _Optional[int] = ...) -> None: ...

class ListTelemetrySummaryResponse(_message.Message):
    __slots__ = ("rows", "total_motes", "total_output_tokens")
    ROWS_FIELD_NUMBER: _ClassVar[int]
    TOTAL_MOTES_FIELD_NUMBER: _ClassVar[int]
    TOTAL_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    rows: _containers.RepeatedCompositeFieldContainer[ModelTokenRollup]
    total_motes: int
    total_output_tokens: int
    def __init__(self, rows: _Optional[_Iterable[_Union[ModelTokenRollup, _Mapping]]] = ..., total_motes: _Optional[int] = ..., total_output_tokens: _Optional[int] = ...) -> None: ...

class SubmitFeedbackRequest(_message.Message):
    __slots__ = ("rating", "message_id", "instance_id", "mote_id", "content_ref", "comment", "recipe_handle", "model_id")
    RATING_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    COMMENT_FIELD_NUMBER: _ClassVar[int]
    RECIPE_HANDLE_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    rating: FeedbackRating
    message_id: str
    instance_id: bytes
    mote_id: bytes
    content_ref: bytes
    comment: str
    recipe_handle: str
    model_id: str
    def __init__(self, rating: _Optional[_Union[FeedbackRating, str]] = ..., message_id: _Optional[str] = ..., instance_id: _Optional[bytes] = ..., mote_id: _Optional[bytes] = ..., content_ref: _Optional[bytes] = ..., comment: _Optional[str] = ..., recipe_handle: _Optional[str] = ..., model_id: _Optional[str] = ...) -> None: ...

class SubmitFeedbackResponse(_message.Message):
    __slots__ = ("feedback_id",)
    FEEDBACK_ID_FIELD_NUMBER: _ClassVar[int]
    feedback_id: bytes
    def __init__(self, feedback_id: _Optional[bytes] = ...) -> None: ...

class ListFeedbackRequest(_message.Message):
    __slots__ = ("limit", "instance_id", "before_rowid")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    BEFORE_ROWID_FIELD_NUMBER: _ClassVar[int]
    limit: int
    instance_id: bytes
    before_rowid: int
    def __init__(self, limit: _Optional[int] = ..., instance_id: _Optional[bytes] = ..., before_rowid: _Optional[int] = ...) -> None: ...

class FeedbackRow(_message.Message):
    __slots__ = ("feedback_id", "rating", "message_id", "instance_id", "mote_id", "content_ref", "comment", "recipe_handle", "model_id", "submitted_unix_ms", "rowid")
    FEEDBACK_ID_FIELD_NUMBER: _ClassVar[int]
    RATING_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    COMMENT_FIELD_NUMBER: _ClassVar[int]
    RECIPE_HANDLE_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    SUBMITTED_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    ROWID_FIELD_NUMBER: _ClassVar[int]
    feedback_id: bytes
    rating: FeedbackRating
    message_id: str
    instance_id: bytes
    mote_id: bytes
    content_ref: bytes
    comment: str
    recipe_handle: str
    model_id: str
    submitted_unix_ms: int
    rowid: int
    def __init__(self, feedback_id: _Optional[bytes] = ..., rating: _Optional[_Union[FeedbackRating, str]] = ..., message_id: _Optional[str] = ..., instance_id: _Optional[bytes] = ..., mote_id: _Optional[bytes] = ..., content_ref: _Optional[bytes] = ..., comment: _Optional[str] = ..., recipe_handle: _Optional[str] = ..., model_id: _Optional[str] = ..., submitted_unix_ms: _Optional[int] = ..., rowid: _Optional[int] = ...) -> None: ...

class ListFeedbackResponse(_message.Message):
    __slots__ = ("rows", "has_more")
    ROWS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    rows: _containers.RepeatedCompositeFieldContainer[FeedbackRow]
    has_more: bool
    def __init__(self, rows: _Optional[_Iterable[_Union[FeedbackRow, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class StreamModelTokensRequest(_message.Message):
    __slots__ = ("instance_id", "mote_id", "since_seq")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    SINCE_SEQ_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    mote_id: bytes
    since_seq: int
    def __init__(self, instance_id: _Optional[bytes] = ..., mote_id: _Optional[bytes] = ..., since_seq: _Optional[int] = ...) -> None: ...

class TokenChunk(_message.Message):
    __slots__ = ("seq", "mote_id", "text_piece", "done")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    TEXT_PIECE_FIELD_NUMBER: _ClassVar[int]
    DONE_FIELD_NUMBER: _ClassVar[int]
    seq: int
    mote_id: bytes
    text_piece: bytes
    done: bool
    def __init__(self, seq: _Optional[int] = ..., mote_id: _Optional[bytes] = ..., text_piece: _Optional[bytes] = ..., done: bool = ...) -> None: ...

class ListAlertsRequest(_message.Message):
    __slots__ = ("limit", "instance_id", "before_seq")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    BEFORE_SEQ_FIELD_NUMBER: _ClassVar[int]
    limit: int
    instance_id: bytes
    before_seq: int
    def __init__(self, limit: _Optional[int] = ..., instance_id: _Optional[bytes] = ..., before_seq: _Optional[int] = ...) -> None: ...

class AlertSummary(_message.Message):
    __slots__ = ("alert_id", "mote_id", "instance_id", "reason_class", "severity", "seq", "created_unix_ms", "reason_code")
    ALERT_ID_FIELD_NUMBER: _ClassVar[int]
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    REASON_CLASS_FIELD_NUMBER: _ClassVar[int]
    SEVERITY_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    CREATED_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    REASON_CODE_FIELD_NUMBER: _ClassVar[int]
    alert_id: bytes
    mote_id: bytes
    instance_id: bytes
    reason_class: str
    severity: str
    seq: int
    created_unix_ms: int
    reason_code: int
    def __init__(self, alert_id: _Optional[bytes] = ..., mote_id: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ..., reason_class: _Optional[str] = ..., severity: _Optional[str] = ..., seq: _Optional[int] = ..., created_unix_ms: _Optional[int] = ..., reason_code: _Optional[int] = ...) -> None: ...

class ListAlertsResponse(_message.Message):
    __slots__ = ("alerts", "has_more")
    ALERTS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    alerts: _containers.RepeatedCompositeFieldContainer[AlertSummary]
    has_more: bool
    def __init__(self, alerts: _Optional[_Iterable[_Union[AlertSummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class ToolParamSpec(_message.Message):
    __slots__ = ("name", "ty", "max_len", "required", "allowed")
    NAME_FIELD_NUMBER: _ClassVar[int]
    TY_FIELD_NUMBER: _ClassVar[int]
    MAX_LEN_FIELD_NUMBER: _ClassVar[int]
    REQUIRED_FIELD_NUMBER: _ClassVar[int]
    ALLOWED_FIELD_NUMBER: _ClassVar[int]
    name: str
    ty: str
    max_len: int
    required: bool
    allowed: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, name: _Optional[str] = ..., ty: _Optional[str] = ..., max_len: _Optional[int] = ..., required: bool = ..., allowed: _Optional[_Iterable[str]] = ...) -> None: ...

class ToolInputSchema(_message.Message):
    __slots__ = ("params", "deny_unknown")
    PARAMS_FIELD_NUMBER: _ClassVar[int]
    DENY_UNKNOWN_FIELD_NUMBER: _ClassVar[int]
    params: _containers.RepeatedCompositeFieldContainer[ToolParamSpec]
    deny_unknown: bool
    def __init__(self, params: _Optional[_Iterable[_Union[ToolParamSpec, _Mapping]]] = ..., deny_unknown: bool = ...) -> None: ...

class RegisterToolRequest(_message.Message):
    __slots__ = ("tool_name", "tool_version", "description", "idempotency_class", "input_schema", "server_host", "remote_name")
    TOOL_NAME_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_CLASS_FIELD_NUMBER: _ClassVar[int]
    INPUT_SCHEMA_FIELD_NUMBER: _ClassVar[int]
    SERVER_HOST_FIELD_NUMBER: _ClassVar[int]
    REMOTE_NAME_FIELD_NUMBER: _ClassVar[int]
    tool_name: str
    tool_version: str
    description: str
    idempotency_class: str
    input_schema: ToolInputSchema
    server_host: str
    remote_name: str
    def __init__(self, tool_name: _Optional[str] = ..., tool_version: _Optional[str] = ..., description: _Optional[str] = ..., idempotency_class: _Optional[str] = ..., input_schema: _Optional[_Union[ToolInputSchema, _Mapping]] = ..., server_host: _Optional[str] = ..., remote_name: _Optional[str] = ...) -> None: ...

class RegisterToolResponse(_message.Message):
    __slots__ = ("tool_id", "registration_status")
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    REGISTRATION_STATUS_FIELD_NUMBER: _ClassVar[int]
    tool_id: bytes
    registration_status: str
    def __init__(self, tool_id: _Optional[bytes] = ..., registration_status: _Optional[str] = ...) -> None: ...

class DeregisterToolRequest(_message.Message):
    __slots__ = ("tool_name", "tool_version")
    TOOL_NAME_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    tool_name: str
    tool_version: str
    def __init__(self, tool_name: _Optional[str] = ..., tool_version: _Optional[str] = ...) -> None: ...

class DeregisterToolResponse(_message.Message):
    __slots__ = ("removed",)
    REMOVED_FIELD_NUMBER: _ClassVar[int]
    removed: bool
    def __init__(self, removed: bool = ...) -> None: ...

class DiscoverToolsRequest(_message.Message):
    __slots__ = ("limit", "after_name", "after_version")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    AFTER_NAME_FIELD_NUMBER: _ClassVar[int]
    AFTER_VERSION_FIELD_NUMBER: _ClassVar[int]
    limit: int
    after_name: str
    after_version: str
    def __init__(self, limit: _Optional[int] = ..., after_name: _Optional[str] = ..., after_version: _Optional[str] = ...) -> None: ...

class RegisteredTool(_message.Message):
    __slots__ = ("tool_id", "tool_name", "tool_version", "kind", "description", "idempotency_class", "provenance", "registration_status", "server_host", "net_scope_summary", "is_builtin")
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_NAME_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_CLASS_FIELD_NUMBER: _ClassVar[int]
    PROVENANCE_FIELD_NUMBER: _ClassVar[int]
    REGISTRATION_STATUS_FIELD_NUMBER: _ClassVar[int]
    SERVER_HOST_FIELD_NUMBER: _ClassVar[int]
    NET_SCOPE_SUMMARY_FIELD_NUMBER: _ClassVar[int]
    IS_BUILTIN_FIELD_NUMBER: _ClassVar[int]
    tool_id: bytes
    tool_name: str
    tool_version: str
    kind: str
    description: str
    idempotency_class: str
    provenance: str
    registration_status: str
    server_host: str
    net_scope_summary: str
    is_builtin: bool
    def __init__(self, tool_id: _Optional[bytes] = ..., tool_name: _Optional[str] = ..., tool_version: _Optional[str] = ..., kind: _Optional[str] = ..., description: _Optional[str] = ..., idempotency_class: _Optional[str] = ..., provenance: _Optional[str] = ..., registration_status: _Optional[str] = ..., server_host: _Optional[str] = ..., net_scope_summary: _Optional[str] = ..., is_builtin: bool = ...) -> None: ...

class DiscoverToolsResponse(_message.Message):
    __slots__ = ("tools", "has_more")
    TOOLS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    tools: _containers.RepeatedCompositeFieldContainer[RegisteredTool]
    has_more: bool
    def __init__(self, tools: _Optional[_Iterable[_Union[RegisteredTool, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class RegisterMcpServerRequest(_message.Message):
    __slots__ = ("server_name", "transport", "endpoint", "args", "tls_required", "credential_ref", "session_mode")
    SERVER_NAME_FIELD_NUMBER: _ClassVar[int]
    TRANSPORT_FIELD_NUMBER: _ClassVar[int]
    ENDPOINT_FIELD_NUMBER: _ClassVar[int]
    ARGS_FIELD_NUMBER: _ClassVar[int]
    TLS_REQUIRED_FIELD_NUMBER: _ClassVar[int]
    CREDENTIAL_REF_FIELD_NUMBER: _ClassVar[int]
    SESSION_MODE_FIELD_NUMBER: _ClassVar[int]
    server_name: str
    transport: str
    endpoint: str
    args: _containers.RepeatedScalarFieldContainer[str]
    tls_required: bool
    credential_ref: str
    session_mode: str
    def __init__(self, server_name: _Optional[str] = ..., transport: _Optional[str] = ..., endpoint: _Optional[str] = ..., args: _Optional[_Iterable[str]] = ..., tls_required: bool = ..., credential_ref: _Optional[str] = ..., session_mode: _Optional[str] = ...) -> None: ...

class RegisterMcpServerResponse(_message.Message):
    __slots__ = ("connection_id", "discovered", "health")
    CONNECTION_ID_FIELD_NUMBER: _ClassVar[int]
    DISCOVERED_FIELD_NUMBER: _ClassVar[int]
    HEALTH_FIELD_NUMBER: _ClassVar[int]
    connection_id: bytes
    discovered: int
    health: str
    def __init__(self, connection_id: _Optional[bytes] = ..., discovered: _Optional[int] = ..., health: _Optional[str] = ...) -> None: ...

class ListMcpServersRequest(_message.Message):
    __slots__ = ("limit", "after_name")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    AFTER_NAME_FIELD_NUMBER: _ClassVar[int]
    limit: int
    after_name: str
    def __init__(self, limit: _Optional[int] = ..., after_name: _Optional[str] = ...) -> None: ...

class McpServer(_message.Message):
    __slots__ = ("connection_id", "server_name", "transport", "endpoint", "health", "tool_count", "credential_ref_present", "session_mode")
    CONNECTION_ID_FIELD_NUMBER: _ClassVar[int]
    SERVER_NAME_FIELD_NUMBER: _ClassVar[int]
    TRANSPORT_FIELD_NUMBER: _ClassVar[int]
    ENDPOINT_FIELD_NUMBER: _ClassVar[int]
    HEALTH_FIELD_NUMBER: _ClassVar[int]
    TOOL_COUNT_FIELD_NUMBER: _ClassVar[int]
    CREDENTIAL_REF_PRESENT_FIELD_NUMBER: _ClassVar[int]
    SESSION_MODE_FIELD_NUMBER: _ClassVar[int]
    connection_id: bytes
    server_name: str
    transport: str
    endpoint: str
    health: str
    tool_count: int
    credential_ref_present: bool
    session_mode: str
    def __init__(self, connection_id: _Optional[bytes] = ..., server_name: _Optional[str] = ..., transport: _Optional[str] = ..., endpoint: _Optional[str] = ..., health: _Optional[str] = ..., tool_count: _Optional[int] = ..., credential_ref_present: bool = ..., session_mode: _Optional[str] = ...) -> None: ...

class ListMcpServersResponse(_message.Message):
    __slots__ = ("servers", "has_more")
    SERVERS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    servers: _containers.RepeatedCompositeFieldContainer[McpServer]
    has_more: bool
    def __init__(self, servers: _Optional[_Iterable[_Union[McpServer, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class DiscoverServerToolsRequest(_message.Message):
    __slots__ = ("server_name",)
    SERVER_NAME_FIELD_NUMBER: _ClassVar[int]
    server_name: str
    def __init__(self, server_name: _Optional[str] = ...) -> None: ...

class DiscoverServerToolsResponse(_message.Message):
    __slots__ = ("tools", "discovered")
    TOOLS_FIELD_NUMBER: _ClassVar[int]
    DISCOVERED_FIELD_NUMBER: _ClassVar[int]
    tools: _containers.RepeatedCompositeFieldContainer[RegisteredTool]
    discovered: int
    def __init__(self, tools: _Optional[_Iterable[_Union[RegisteredTool, _Mapping]]] = ..., discovered: _Optional[int] = ...) -> None: ...

class TestMcpServerRequest(_message.Message):
    __slots__ = ("server_name",)
    SERVER_NAME_FIELD_NUMBER: _ClassVar[int]
    server_name: str
    def __init__(self, server_name: _Optional[str] = ...) -> None: ...

class TestMcpServerResponse(_message.Message):
    __slots__ = ("reachable", "detail")
    REACHABLE_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    reachable: bool
    detail: str
    def __init__(self, reachable: bool = ..., detail: _Optional[str] = ...) -> None: ...

class DeregisterMcpServerRequest(_message.Message):
    __slots__ = ("server_name",)
    SERVER_NAME_FIELD_NUMBER: _ClassVar[int]
    server_name: str
    def __init__(self, server_name: _Optional[str] = ...) -> None: ...

class DeregisterMcpServerResponse(_message.Message):
    __slots__ = ("removed",)
    REMOVED_FIELD_NUMBER: _ClassVar[int]
    removed: bool
    def __init__(self, removed: bool = ...) -> None: ...

class CallMcpToolRequest(_message.Message):
    __slots__ = ("server_name", "remote_name", "args_json")
    SERVER_NAME_FIELD_NUMBER: _ClassVar[int]
    REMOTE_NAME_FIELD_NUMBER: _ClassVar[int]
    ARGS_JSON_FIELD_NUMBER: _ClassVar[int]
    server_name: str
    remote_name: str
    args_json: str
    def __init__(self, server_name: _Optional[str] = ..., remote_name: _Optional[str] = ..., args_json: _Optional[str] = ...) -> None: ...

class CallMcpToolResponse(_message.Message):
    __slots__ = ("ok", "result_json", "error")
    OK_FIELD_NUMBER: _ClassVar[int]
    RESULT_JSON_FIELD_NUMBER: _ClassVar[int]
    ERROR_FIELD_NUMBER: _ClassVar[int]
    ok: bool
    result_json: str
    error: str
    def __init__(self, ok: bool = ..., result_json: _Optional[str] = ..., error: _Optional[str] = ...) -> None: ...

class ContextItem(_message.Message):
    __slots__ = ("name", "content_ref", "media_type")
    NAME_FIELD_NUMBER: _ClassVar[int]
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    MEDIA_TYPE_FIELD_NUMBER: _ClassVar[int]
    name: str
    content_ref: bytes
    media_type: str
    def __init__(self, name: _Optional[str] = ..., content_ref: _Optional[bytes] = ..., media_type: _Optional[str] = ...) -> None: ...

class PutContextBundleRequest(_message.Message):
    __slots__ = ("handle", "description", "items")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    ITEMS_FIELD_NUMBER: _ClassVar[int]
    handle: str
    description: str
    items: _containers.RepeatedCompositeFieldContainer[ContextItem]
    def __init__(self, handle: _Optional[str] = ..., description: _Optional[str] = ..., items: _Optional[_Iterable[_Union[ContextItem, _Mapping]]] = ...) -> None: ...

class PutContextBundleResponse(_message.Message):
    __slots__ = ("bundle_ref", "handle", "deduplicated")
    BUNDLE_REF_FIELD_NUMBER: _ClassVar[int]
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    DEDUPLICATED_FIELD_NUMBER: _ClassVar[int]
    bundle_ref: bytes
    handle: str
    deduplicated: bool
    def __init__(self, bundle_ref: _Optional[bytes] = ..., handle: _Optional[str] = ..., deduplicated: bool = ...) -> None: ...

class ListContextBundlesRequest(_message.Message):
    __slots__ = ("limit", "after_handle")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    AFTER_HANDLE_FIELD_NUMBER: _ClassVar[int]
    limit: int
    after_handle: str
    def __init__(self, limit: _Optional[int] = ..., after_handle: _Optional[str] = ...) -> None: ...

class ContextBundle(_message.Message):
    __slots__ = ("bundle_ref", "handle", "description", "items", "item_count")
    BUNDLE_REF_FIELD_NUMBER: _ClassVar[int]
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    ITEMS_FIELD_NUMBER: _ClassVar[int]
    ITEM_COUNT_FIELD_NUMBER: _ClassVar[int]
    bundle_ref: bytes
    handle: str
    description: str
    items: _containers.RepeatedCompositeFieldContainer[ContextItem]
    item_count: int
    def __init__(self, bundle_ref: _Optional[bytes] = ..., handle: _Optional[str] = ..., description: _Optional[str] = ..., items: _Optional[_Iterable[_Union[ContextItem, _Mapping]]] = ..., item_count: _Optional[int] = ...) -> None: ...

class ListContextBundlesResponse(_message.Message):
    __slots__ = ("bundles", "has_more")
    BUNDLES_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    bundles: _containers.RepeatedCompositeFieldContainer[ContextBundle]
    has_more: bool
    def __init__(self, bundles: _Optional[_Iterable[_Union[ContextBundle, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class GetContextBundleRequest(_message.Message):
    __slots__ = ("handle",)
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    def __init__(self, handle: _Optional[str] = ...) -> None: ...

class GetContextBundleResponse(_message.Message):
    __slots__ = ("bundle", "found")
    BUNDLE_FIELD_NUMBER: _ClassVar[int]
    FOUND_FIELD_NUMBER: _ClassVar[int]
    bundle: ContextBundle
    found: bool
    def __init__(self, bundle: _Optional[_Union[ContextBundle, _Mapping]] = ..., found: bool = ...) -> None: ...

class DeleteContextBundleRequest(_message.Message):
    __slots__ = ("handle",)
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    def __init__(self, handle: _Optional[str] = ...) -> None: ...

class DeleteContextBundleResponse(_message.Message):
    __slots__ = ("removed",)
    REMOVED_FIELD_NUMBER: _ClassVar[int]
    removed: bool
    def __init__(self, removed: bool = ...) -> None: ...

class BranchItem(_message.Message):
    __slots__ = ("path", "content_ref")
    PATH_FIELD_NUMBER: _ClassVar[int]
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    path: str
    content_ref: bytes
    def __init__(self, path: _Optional[str] = ..., content_ref: _Optional[bytes] = ...) -> None: ...

class Branch(_message.Message):
    __slots__ = ("branch_ref", "handle", "parent_handle", "description", "items", "item_count")
    BRANCH_REF_FIELD_NUMBER: _ClassVar[int]
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    PARENT_HANDLE_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    ITEMS_FIELD_NUMBER: _ClassVar[int]
    ITEM_COUNT_FIELD_NUMBER: _ClassVar[int]
    branch_ref: bytes
    handle: str
    parent_handle: str
    description: str
    items: _containers.RepeatedCompositeFieldContainer[BranchItem]
    item_count: int
    def __init__(self, branch_ref: _Optional[bytes] = ..., handle: _Optional[str] = ..., parent_handle: _Optional[str] = ..., description: _Optional[str] = ..., items: _Optional[_Iterable[_Union[BranchItem, _Mapping]]] = ..., item_count: _Optional[int] = ...) -> None: ...

class CreateBranchRequest(_message.Message):
    __slots__ = ("handle", "description", "parent_handle")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    PARENT_HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    description: str
    parent_handle: str
    def __init__(self, handle: _Optional[str] = ..., description: _Optional[str] = ..., parent_handle: _Optional[str] = ...) -> None: ...

class CreateBranchResponse(_message.Message):
    __slots__ = ("branch_ref", "handle", "deduplicated")
    BRANCH_REF_FIELD_NUMBER: _ClassVar[int]
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    DEDUPLICATED_FIELD_NUMBER: _ClassVar[int]
    branch_ref: bytes
    handle: str
    deduplicated: bool
    def __init__(self, branch_ref: _Optional[bytes] = ..., handle: _Optional[str] = ..., deduplicated: bool = ...) -> None: ...

class SnapshotIntoRequest(_message.Message):
    __slots__ = ("handle", "paths", "description", "parent_handle")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    PATHS_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    PARENT_HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    paths: _containers.RepeatedScalarFieldContainer[str]
    description: str
    parent_handle: str
    def __init__(self, handle: _Optional[str] = ..., paths: _Optional[_Iterable[str]] = ..., description: _Optional[str] = ..., parent_handle: _Optional[str] = ...) -> None: ...

class SnapshotIntoResponse(_message.Message):
    __slots__ = ("branch_ref", "handle", "items", "ingested", "deduplicated")
    BRANCH_REF_FIELD_NUMBER: _ClassVar[int]
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    ITEMS_FIELD_NUMBER: _ClassVar[int]
    INGESTED_FIELD_NUMBER: _ClassVar[int]
    DEDUPLICATED_FIELD_NUMBER: _ClassVar[int]
    branch_ref: bytes
    handle: str
    items: _containers.RepeatedCompositeFieldContainer[BranchItem]
    ingested: int
    deduplicated: bool
    def __init__(self, branch_ref: _Optional[bytes] = ..., handle: _Optional[str] = ..., items: _Optional[_Iterable[_Union[BranchItem, _Mapping]]] = ..., ingested: _Optional[int] = ..., deduplicated: bool = ...) -> None: ...

class ListBranchesRequest(_message.Message):
    __slots__ = ("limit", "after_handle")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    AFTER_HANDLE_FIELD_NUMBER: _ClassVar[int]
    limit: int
    after_handle: str
    def __init__(self, limit: _Optional[int] = ..., after_handle: _Optional[str] = ...) -> None: ...

class ListBranchesResponse(_message.Message):
    __slots__ = ("branches", "has_more")
    BRANCHES_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    branches: _containers.RepeatedCompositeFieldContainer[Branch]
    has_more: bool
    def __init__(self, branches: _Optional[_Iterable[_Union[Branch, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class GetBranchRequest(_message.Message):
    __slots__ = ("handle",)
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    def __init__(self, handle: _Optional[str] = ...) -> None: ...

class GetBranchResponse(_message.Message):
    __slots__ = ("branch", "found")
    BRANCH_FIELD_NUMBER: _ClassVar[int]
    FOUND_FIELD_NUMBER: _ClassVar[int]
    branch: Branch
    found: bool
    def __init__(self, branch: _Optional[_Union[Branch, _Mapping]] = ..., found: bool = ...) -> None: ...

class DeleteBranchRequest(_message.Message):
    __slots__ = ("handle",)
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    def __init__(self, handle: _Optional[str] = ...) -> None: ...

class DeleteBranchResponse(_message.Message):
    __slots__ = ("removed",)
    REMOVED_FIELD_NUMBER: _ClassVar[int]
    removed: bool
    def __init__(self, removed: bool = ...) -> None: ...

class AdvanceBranchRequest(_message.Message):
    __slots__ = ("handle", "path", "content_ref")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    PATH_FIELD_NUMBER: _ClassVar[int]
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    handle: str
    path: str
    content_ref: bytes
    def __init__(self, handle: _Optional[str] = ..., path: _Optional[str] = ..., content_ref: _Optional[bytes] = ...) -> None: ...

class AdvanceBranchResponse(_message.Message):
    __slots__ = ("branch_ref", "handle", "items", "deduplicated")
    BRANCH_REF_FIELD_NUMBER: _ClassVar[int]
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    ITEMS_FIELD_NUMBER: _ClassVar[int]
    DEDUPLICATED_FIELD_NUMBER: _ClassVar[int]
    branch_ref: bytes
    handle: str
    items: _containers.RepeatedCompositeFieldContainer[BranchItem]
    deduplicated: bool
    def __init__(self, branch_ref: _Optional[bytes] = ..., handle: _Optional[str] = ..., items: _Optional[_Iterable[_Union[BranchItem, _Mapping]]] = ..., deduplicated: bool = ...) -> None: ...

class GetBranchContentRequest(_message.Message):
    __slots__ = ("handle", "path")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    PATH_FIELD_NUMBER: _ClassVar[int]
    handle: str
    path: str
    def __init__(self, handle: _Optional[str] = ..., path: _Optional[str] = ...) -> None: ...

class GetBranchContentResponse(_message.Message):
    __slots__ = ("payload", "found")
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    FOUND_FIELD_NUMBER: _ClassVar[int]
    payload: bytes
    found: bool
    def __init__(self, payload: _Optional[bytes] = ..., found: bool = ...) -> None: ...

class GetServerInfoRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class GetServerInfoResponse(_message.Message):
    __slots__ = ("model_id", "model_path", "listen_addr", "ws_addr", "console_addr", "metrics_addr", "content_root", "journal_path", "catalog_dir", "max_lease", "content_max_bytes", "cors_origins", "tls_enabled", "auth_mode", "feature_hnsw", "feature_inference", "feature_console", "feature_vision", "audit_log_enabled", "react_max_turns", "react_max_tool_calls", "embed_model_id", "active_model_id", "allow_model_pull", "embed_model_is_decoder")
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    MODEL_PATH_FIELD_NUMBER: _ClassVar[int]
    LISTEN_ADDR_FIELD_NUMBER: _ClassVar[int]
    WS_ADDR_FIELD_NUMBER: _ClassVar[int]
    CONSOLE_ADDR_FIELD_NUMBER: _ClassVar[int]
    METRICS_ADDR_FIELD_NUMBER: _ClassVar[int]
    CONTENT_ROOT_FIELD_NUMBER: _ClassVar[int]
    JOURNAL_PATH_FIELD_NUMBER: _ClassVar[int]
    CATALOG_DIR_FIELD_NUMBER: _ClassVar[int]
    MAX_LEASE_FIELD_NUMBER: _ClassVar[int]
    CONTENT_MAX_BYTES_FIELD_NUMBER: _ClassVar[int]
    CORS_ORIGINS_FIELD_NUMBER: _ClassVar[int]
    TLS_ENABLED_FIELD_NUMBER: _ClassVar[int]
    AUTH_MODE_FIELD_NUMBER: _ClassVar[int]
    FEATURE_HNSW_FIELD_NUMBER: _ClassVar[int]
    FEATURE_INFERENCE_FIELD_NUMBER: _ClassVar[int]
    FEATURE_CONSOLE_FIELD_NUMBER: _ClassVar[int]
    FEATURE_VISION_FIELD_NUMBER: _ClassVar[int]
    AUDIT_LOG_ENABLED_FIELD_NUMBER: _ClassVar[int]
    REACT_MAX_TURNS_FIELD_NUMBER: _ClassVar[int]
    REACT_MAX_TOOL_CALLS_FIELD_NUMBER: _ClassVar[int]
    EMBED_MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    ACTIVE_MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    ALLOW_MODEL_PULL_FIELD_NUMBER: _ClassVar[int]
    EMBED_MODEL_IS_DECODER_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    model_path: str
    listen_addr: str
    ws_addr: str
    console_addr: str
    metrics_addr: str
    content_root: str
    journal_path: str
    catalog_dir: str
    max_lease: int
    content_max_bytes: int
    cors_origins: _containers.RepeatedScalarFieldContainer[str]
    tls_enabled: bool
    auth_mode: str
    feature_hnsw: bool
    feature_inference: bool
    feature_console: bool
    feature_vision: bool
    audit_log_enabled: bool
    react_max_turns: int
    react_max_tool_calls: int
    embed_model_id: str
    active_model_id: str
    allow_model_pull: bool
    embed_model_is_decoder: bool
    def __init__(self, model_id: _Optional[str] = ..., model_path: _Optional[str] = ..., listen_addr: _Optional[str] = ..., ws_addr: _Optional[str] = ..., console_addr: _Optional[str] = ..., metrics_addr: _Optional[str] = ..., content_root: _Optional[str] = ..., journal_path: _Optional[str] = ..., catalog_dir: _Optional[str] = ..., max_lease: _Optional[int] = ..., content_max_bytes: _Optional[int] = ..., cors_origins: _Optional[_Iterable[str]] = ..., tls_enabled: bool = ..., auth_mode: _Optional[str] = ..., feature_hnsw: bool = ..., feature_inference: bool = ..., feature_console: bool = ..., feature_vision: bool = ..., audit_log_enabled: bool = ..., react_max_turns: _Optional[int] = ..., react_max_tool_calls: _Optional[int] = ..., embed_model_id: _Optional[str] = ..., active_model_id: _Optional[str] = ..., allow_model_pull: bool = ..., embed_model_is_decoder: bool = ...) -> None: ...

class PutSecretRequest(_message.Message):
    __slots__ = ("name", "value")
    NAME_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    name: str
    value: str
    def __init__(self, name: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...

class PutSecretResponse(_message.Message):
    __slots__ = ("stored",)
    STORED_FIELD_NUMBER: _ClassVar[int]
    stored: bool
    def __init__(self, stored: bool = ...) -> None: ...

class ListSecretNamesRequest(_message.Message):
    __slots__ = ("limit", "after_name")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    AFTER_NAME_FIELD_NUMBER: _ClassVar[int]
    limit: int
    after_name: str
    def __init__(self, limit: _Optional[int] = ..., after_name: _Optional[str] = ...) -> None: ...

class SecretName(_message.Message):
    __slots__ = ("name", "created_unix_ms", "updated_unix_ms")
    NAME_FIELD_NUMBER: _ClassVar[int]
    CREATED_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    UPDATED_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    name: str
    created_unix_ms: int
    updated_unix_ms: int
    def __init__(self, name: _Optional[str] = ..., created_unix_ms: _Optional[int] = ..., updated_unix_ms: _Optional[int] = ...) -> None: ...

class ListSecretNamesResponse(_message.Message):
    __slots__ = ("names", "has_more")
    NAMES_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    names: _containers.RepeatedCompositeFieldContainer[SecretName]
    has_more: bool
    def __init__(self, names: _Optional[_Iterable[_Union[SecretName, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class DeleteSecretRequest(_message.Message):
    __slots__ = ("name",)
    NAME_FIELD_NUMBER: _ClassVar[int]
    name: str
    def __init__(self, name: _Optional[str] = ...) -> None: ...

class DeleteSecretResponse(_message.Message):
    __slots__ = ("removed",)
    REMOVED_FIELD_NUMBER: _ClassVar[int]
    removed: bool
    def __init__(self, removed: bool = ...) -> None: ...

class RegisterTriggerRequest(_message.Message):
    __slots__ = ("name", "kind", "recipe_handle", "auth", "auth_secret_ref", "schedule_spec", "enabled")
    NAME_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    RECIPE_HANDLE_FIELD_NUMBER: _ClassVar[int]
    AUTH_FIELD_NUMBER: _ClassVar[int]
    AUTH_SECRET_REF_FIELD_NUMBER: _ClassVar[int]
    SCHEDULE_SPEC_FIELD_NUMBER: _ClassVar[int]
    ENABLED_FIELD_NUMBER: _ClassVar[int]
    name: str
    kind: TriggerKind
    recipe_handle: str
    auth: TriggerAuth
    auth_secret_ref: str
    schedule_spec: str
    enabled: bool
    def __init__(self, name: _Optional[str] = ..., kind: _Optional[_Union[TriggerKind, str]] = ..., recipe_handle: _Optional[str] = ..., auth: _Optional[_Union[TriggerAuth, str]] = ..., auth_secret_ref: _Optional[str] = ..., schedule_spec: _Optional[str] = ..., enabled: bool = ...) -> None: ...

class RegisterTriggerResponse(_message.Message):
    __slots__ = ("trigger_id",)
    TRIGGER_ID_FIELD_NUMBER: _ClassVar[int]
    trigger_id: bytes
    def __init__(self, trigger_id: _Optional[bytes] = ...) -> None: ...

class ListTriggersRequest(_message.Message):
    __slots__ = ("limit", "after_name")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    AFTER_NAME_FIELD_NUMBER: _ClassVar[int]
    limit: int
    after_name: str
    def __init__(self, limit: _Optional[int] = ..., after_name: _Optional[str] = ...) -> None: ...

class TriggerView(_message.Message):
    __slots__ = ("trigger_id", "name", "kind", "recipe_handle", "auth", "auth_secret_present", "schedule_spec", "enabled", "last_fire_unix_ms")
    TRIGGER_ID_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    RECIPE_HANDLE_FIELD_NUMBER: _ClassVar[int]
    AUTH_FIELD_NUMBER: _ClassVar[int]
    AUTH_SECRET_PRESENT_FIELD_NUMBER: _ClassVar[int]
    SCHEDULE_SPEC_FIELD_NUMBER: _ClassVar[int]
    ENABLED_FIELD_NUMBER: _ClassVar[int]
    LAST_FIRE_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    trigger_id: bytes
    name: str
    kind: TriggerKind
    recipe_handle: str
    auth: TriggerAuth
    auth_secret_present: bool
    schedule_spec: str
    enabled: bool
    last_fire_unix_ms: int
    def __init__(self, trigger_id: _Optional[bytes] = ..., name: _Optional[str] = ..., kind: _Optional[_Union[TriggerKind, str]] = ..., recipe_handle: _Optional[str] = ..., auth: _Optional[_Union[TriggerAuth, str]] = ..., auth_secret_present: bool = ..., schedule_spec: _Optional[str] = ..., enabled: bool = ..., last_fire_unix_ms: _Optional[int] = ...) -> None: ...

class ListTriggersResponse(_message.Message):
    __slots__ = ("triggers", "has_more")
    TRIGGERS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    triggers: _containers.RepeatedCompositeFieldContainer[TriggerView]
    has_more: bool
    def __init__(self, triggers: _Optional[_Iterable[_Union[TriggerView, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

class DeregisterTriggerRequest(_message.Message):
    __slots__ = ("name",)
    NAME_FIELD_NUMBER: _ClassVar[int]
    name: str
    def __init__(self, name: _Optional[str] = ...) -> None: ...

class DeregisterTriggerResponse(_message.Message):
    __slots__ = ("removed",)
    REMOVED_FIELD_NUMBER: _ClassVar[int]
    removed: bool
    def __init__(self, removed: bool = ...) -> None: ...

class SubmitTriggerRequest(_message.Message):
    __slots__ = ("name", "idempotency_key", "payload_json")
    NAME_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_JSON_FIELD_NUMBER: _ClassVar[int]
    name: str
    idempotency_key: str
    payload_json: str
    def __init__(self, name: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., payload_json: _Optional[str] = ...) -> None: ...

class SubmitTriggerResponse(_message.Message):
    __slots__ = ("instance_id", "deduped")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    DEDUPED_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    deduped: bool
    def __init__(self, instance_id: _Optional[bytes] = ..., deduped: bool = ...) -> None: ...

class TestTriggerRequest(_message.Message):
    __slots__ = ("name", "payload_json")
    NAME_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_JSON_FIELD_NUMBER: _ClassVar[int]
    name: str
    payload_json: str
    def __init__(self, name: _Optional[str] = ..., payload_json: _Optional[str] = ...) -> None: ...

class TestTriggerResponse(_message.Message):
    __slots__ = ("ok", "detail")
    OK_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    ok: bool
    detail: str
    def __init__(self, ok: bool = ..., detail: _Optional[str] = ...) -> None: ...

class ListPendingApprovalsRequest(_message.Message):
    __slots__ = ("limit",)
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    limit: int
    def __init__(self, limit: _Optional[int] = ...) -> None: ...

class PendingApproval(_message.Message):
    __slots__ = ("request_id", "instance_id", "mote_id", "tool_id", "tool_version", "intent", "deadline_unix_ms", "created_unix_ms")
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    INTENT_FIELD_NUMBER: _ClassVar[int]
    DEADLINE_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    CREATED_UNIX_MS_FIELD_NUMBER: _ClassVar[int]
    request_id: bytes
    instance_id: bytes
    mote_id: bytes
    tool_id: str
    tool_version: str
    intent: str
    deadline_unix_ms: int
    created_unix_ms: int
    def __init__(self, request_id: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ..., mote_id: _Optional[bytes] = ..., tool_id: _Optional[str] = ..., tool_version: _Optional[str] = ..., intent: _Optional[str] = ..., deadline_unix_ms: _Optional[int] = ..., created_unix_ms: _Optional[int] = ...) -> None: ...

class ListPendingApprovalsResponse(_message.Message):
    __slots__ = ("approvals",)
    APPROVALS_FIELD_NUMBER: _ClassVar[int]
    approvals: _containers.RepeatedCompositeFieldContainer[PendingApproval]
    def __init__(self, approvals: _Optional[_Iterable[_Union[PendingApproval, _Mapping]]] = ...) -> None: ...

class GrantApprovalRequest(_message.Message):
    __slots__ = ("request_id", "reason")
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    request_id: bytes
    reason: str
    def __init__(self, request_id: _Optional[bytes] = ..., reason: _Optional[str] = ...) -> None: ...

class GrantApprovalResponse(_message.Message):
    __slots__ = ("granted",)
    GRANTED_FIELD_NUMBER: _ClassVar[int]
    granted: bool
    def __init__(self, granted: bool = ...) -> None: ...

class DenyApprovalRequest(_message.Message):
    __slots__ = ("request_id", "reason")
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    request_id: bytes
    reason: str
    def __init__(self, request_id: _Optional[bytes] = ..., reason: _Optional[str] = ...) -> None: ...

class DenyApprovalResponse(_message.Message):
    __slots__ = ("denied",)
    DENIED_FIELD_NUMBER: _ClassVar[int]
    denied: bool
    def __init__(self, denied: bool = ...) -> None: ...

class GetRunCostRequest(_message.Message):
    __slots__ = ("instance_id",)
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    def __init__(self, instance_id: _Optional[bytes] = ...) -> None: ...

class GetRunCostResponse(_message.Message):
    __slots__ = ("instance_id", "turns", "tool_calls", "estimated_micro_usd", "ceiling_micro_usd", "per_turn_micro_usd", "per_tool_call_micro_usd", "over_ceiling")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    TURNS_FIELD_NUMBER: _ClassVar[int]
    TOOL_CALLS_FIELD_NUMBER: _ClassVar[int]
    ESTIMATED_MICRO_USD_FIELD_NUMBER: _ClassVar[int]
    CEILING_MICRO_USD_FIELD_NUMBER: _ClassVar[int]
    PER_TURN_MICRO_USD_FIELD_NUMBER: _ClassVar[int]
    PER_TOOL_CALL_MICRO_USD_FIELD_NUMBER: _ClassVar[int]
    OVER_CEILING_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    turns: int
    tool_calls: int
    estimated_micro_usd: int
    ceiling_micro_usd: int
    per_turn_micro_usd: int
    per_tool_call_micro_usd: int
    over_ceiling: bool
    def __init__(self, instance_id: _Optional[bytes] = ..., turns: _Optional[int] = ..., tool_calls: _Optional[int] = ..., estimated_micro_usd: _Optional[int] = ..., ceiling_micro_usd: _Optional[int] = ..., per_turn_micro_usd: _Optional[int] = ..., per_tool_call_micro_usd: _Optional[int] = ..., over_ceiling: bool = ...) -> None: ...

class ScoreRunRequest(_message.Message):
    __slots__ = ("instance_id",)
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    def __init__(self, instance_id: _Optional[bytes] = ...) -> None: ...

class RunScore(_message.Message):
    __slots__ = ("instance_id", "terminal", "reached_answer", "turns_used", "tool_calls_used", "max_turns", "max_tool_calls", "rejections", "turn_budget_used_per_mille", "tool_budget_used_per_mille")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    TERMINAL_FIELD_NUMBER: _ClassVar[int]
    REACHED_ANSWER_FIELD_NUMBER: _ClassVar[int]
    TURNS_USED_FIELD_NUMBER: _ClassVar[int]
    TOOL_CALLS_USED_FIELD_NUMBER: _ClassVar[int]
    MAX_TURNS_FIELD_NUMBER: _ClassVar[int]
    MAX_TOOL_CALLS_FIELD_NUMBER: _ClassVar[int]
    REJECTIONS_FIELD_NUMBER: _ClassVar[int]
    TURN_BUDGET_USED_PER_MILLE_FIELD_NUMBER: _ClassVar[int]
    TOOL_BUDGET_USED_PER_MILLE_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    terminal: str
    reached_answer: bool
    turns_used: int
    tool_calls_used: int
    max_turns: int
    max_tool_calls: int
    rejections: int
    turn_budget_used_per_mille: int
    tool_budget_used_per_mille: int
    def __init__(self, instance_id: _Optional[bytes] = ..., terminal: _Optional[str] = ..., reached_answer: bool = ..., turns_used: _Optional[int] = ..., tool_calls_used: _Optional[int] = ..., max_turns: _Optional[int] = ..., max_tool_calls: _Optional[int] = ..., rejections: _Optional[int] = ..., turn_budget_used_per_mille: _Optional[int] = ..., tool_budget_used_per_mille: _Optional[int] = ...) -> None: ...
