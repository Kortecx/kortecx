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

class WorkflowExecutionMode(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    WORKFLOW_EXECUTION_MODE_UNSPECIFIED: _ClassVar[WorkflowExecutionMode]
    WORKFLOW_EXECUTION_MODE_FROZEN: _ClassVar[WorkflowExecutionMode]
    WORKFLOW_EXECUTION_MODE_DYNAMIC: _ClassVar[WorkflowExecutionMode]
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
LOWER_VERDICT_UNSPECIFIED: LowerVerdict
LOWER_VERDICT_UNAVAILABLE: LowerVerdict
LOWER_VERDICT_WOULD_LOWER: LowerVerdict
LOWER_VERDICT_REFUSED: LowerVerdict
WORKFLOW_STEP_KIND_UNSPECIFIED: WorkflowStepKind
WORKFLOW_STEP_KIND_PURE: WorkflowStepKind
WORKFLOW_STEP_KIND_MODEL: WorkflowStepKind
WORKFLOW_STEP_KIND_EXEC: WorkflowStepKind
WORKFLOW_EXECUTION_MODE_UNSPECIFIED: WorkflowExecutionMode
WORKFLOW_EXECUTION_MODE_FROZEN: WorkflowExecutionMode
WORKFLOW_EXECUTION_MODE_DYNAMIC: WorkflowExecutionMode

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
    __slots__ = ("handle", "args")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    ARGS_FIELD_NUMBER: _ClassVar[int]
    handle: str
    args: bytes
    def __init__(self, handle: _Optional[str] = ..., args: _Optional[bytes] = ...) -> None: ...

class InvokeResponse(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint", "terminal_mote_id")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    TERMINAL_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    terminal_mote_id: bytes
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ..., terminal_mote_id: _Optional[bytes] = ...) -> None: ...

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

class ListRecipesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class RecipeSummary(_message.Message):
    __slots__ = ("handle",)
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    handle: str
    def __init__(self, handle: _Optional[str] = ...) -> None: ...

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
    __slots__ = ("dataset_id", "name", "doc_count", "dim", "created_ms")
    DATASET_ID_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    DOC_COUNT_FIELD_NUMBER: _ClassVar[int]
    DIM_FIELD_NUMBER: _ClassVar[int]
    CREATED_MS_FIELD_NUMBER: _ClassVar[int]
    dataset_id: str
    name: str
    doc_count: int
    dim: int
    created_ms: int
    def __init__(self, dataset_id: _Optional[str] = ..., name: _Optional[str] = ..., doc_count: _Optional[int] = ..., dim: _Optional[int] = ..., created_ms: _Optional[int] = ...) -> None: ...

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
    __slots__ = ("dataset", "query_text", "query_embedding", "k")
    DATASET_FIELD_NUMBER: _ClassVar[int]
    QUERY_TEXT_FIELD_NUMBER: _ClassVar[int]
    QUERY_EMBEDDING_FIELD_NUMBER: _ClassVar[int]
    K_FIELD_NUMBER: _ClassVar[int]
    dataset: str
    query_text: str
    query_embedding: _containers.RepeatedScalarFieldContainer[float]
    k: int
    def __init__(self, dataset: _Optional[str] = ..., query_text: _Optional[str] = ..., query_embedding: _Optional[_Iterable[float]] = ..., k: _Optional[int] = ...) -> None: ...

class DatasetHit(_message.Message):
    __slots__ = ("content_ref", "content", "score")
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    SCORE_FIELD_NUMBER: _ClassVar[int]
    content_ref: bytes
    content: bytes
    score: float
    def __init__(self, content_ref: _Optional[bytes] = ..., content: _Optional[bytes] = ..., score: _Optional[float] = ...) -> None: ...

class QueryDatasetResponse(_message.Message):
    __slots__ = ("hits",)
    HITS_FIELD_NUMBER: _ClassVar[int]
    hits: _containers.RepeatedCompositeFieldContainer[DatasetHit]
    def __init__(self, hits: _Optional[_Iterable[_Union[DatasetHit, _Mapping]]] = ...) -> None: ...

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
    __slots__ = ("limit", "instance_id")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    limit: int
    instance_id: bytes
    def __init__(self, limit: _Optional[int] = ..., instance_id: _Optional[bytes] = ...) -> None: ...

class ReactTurnSummary(_message.Message):
    __slots__ = ("turn", "turn_mote_id", "instance_id", "model_id", "branch", "tool_id", "tool_version", "max_turns", "max_tool_calls", "seq")
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
    def __init__(self, turn: _Optional[int] = ..., turn_mote_id: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ..., model_id: _Optional[str] = ..., branch: _Optional[str] = ..., tool_id: _Optional[str] = ..., tool_version: _Optional[str] = ..., max_turns: _Optional[int] = ..., max_tool_calls: _Optional[int] = ..., seq: _Optional[int] = ...) -> None: ...

class ListReactTurnsResponse(_message.Message):
    __slots__ = ("turns", "has_more")
    TURNS_FIELD_NUMBER: _ClassVar[int]
    HAS_MORE_FIELD_NUMBER: _ClassVar[int]
    turns: _containers.RepeatedCompositeFieldContainer[ReactTurnSummary]
    has_more: bool
    def __init__(self, turns: _Optional[_Iterable[_Union[ReactTurnSummary, _Mapping]]] = ..., has_more: bool = ...) -> None: ...

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
    __slots__ = ("seed", "steps", "edges", "execution_mode")
    SEED_FIELD_NUMBER: _ClassVar[int]
    STEPS_FIELD_NUMBER: _ClassVar[int]
    EDGES_FIELD_NUMBER: _ClassVar[int]
    EXECUTION_MODE_FIELD_NUMBER: _ClassVar[int]
    seed: int
    steps: _containers.RepeatedCompositeFieldContainer[WorkflowStep]
    edges: _containers.RepeatedCompositeFieldContainer[WorkflowEdge]
    execution_mode: WorkflowExecutionMode
    def __init__(self, seed: _Optional[int] = ..., steps: _Optional[_Iterable[_Union[WorkflowStep, _Mapping]]] = ..., edges: _Optional[_Iterable[_Union[WorkflowEdge, _Mapping]]] = ..., execution_mode: _Optional[_Union[WorkflowExecutionMode, str]] = ...) -> None: ...

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
    __slots__ = ("model_id", "modalities", "description", "serving", "context_len")
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    MODALITIES_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    SERVING_FIELD_NUMBER: _ClassVar[int]
    CONTEXT_LEN_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    modalities: _containers.RepeatedScalarFieldContainer[str]
    description: str
    serving: bool
    context_len: int
    def __init__(self, model_id: _Optional[str] = ..., modalities: _Optional[_Iterable[str]] = ..., description: _Optional[str] = ..., serving: bool = ..., context_len: _Optional[int] = ...) -> None: ...

class ListModelsResponse(_message.Message):
    __slots__ = ("models",)
    MODELS_FIELD_NUMBER: _ClassVar[int]
    models: _containers.RepeatedCompositeFieldContainer[ModelSummary]
    def __init__(self, models: _Optional[_Iterable[_Union[ModelSummary, _Mapping]]] = ...) -> None: ...
