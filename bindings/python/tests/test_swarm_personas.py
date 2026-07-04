"""RC-SW2 — swarm/team authoring, the persona library, and the App.run→RunApp fix.

All client-side (no server): the swarm/fan-out/map-reduce methods lower to the same
``[a & b] > g`` topology the golden corpus pins, personas fold into an agent's prompt,
and ``App.run`` routes through ``SaveApp`` + ``RunApp`` (never a local ``submit_workflow``
recompile — the regression that dropped ``references.connections`` + ``secret_scope``).
"""

from __future__ import annotations

import pytest

import kortecx as kx
from kortecx.chains import ChainError

# ---- swarm / team / fan_out_gather / map_reduce lowering (client-side) ----


def test_swarm_lowers_to_parallel_agentic_leaves_then_synthesizer() -> None:
    f = kx.swarm(
        ("Analyze the market", ["mcp-echo/echo"]),
        ("Critique the analysis", ["mcp-echo/echo"]),
        goal="the Q3 plan",
    )
    low = f.lowering()
    assert len(low["steps"]) == 3, "2 agentic leaves + 1 synthesizer"
    # the two leaves are agentic MODEL steps (tool_contract set), goal appended.
    for i in (0, 1):
        assert low["steps"][i]["kind"] == "model"
        assert low["steps"][i]["tool_contract"] == {"mcp-echo/echo": "1"}
        assert low["steps"][i]["prompt"].endswith("the Q3 plan")
    # the gather is a plain MODEL synthesizer (no tools) that fans in both leaves.
    assert low["steps"][2]["kind"] == "model"
    assert low["steps"][2]["tool_contract"] == {}
    assert low["edges"] == [
        {"parent": 0, "child": 2, "edge": "data"},
        {"parent": 1, "child": 2, "edge": "data"},
    ]


def test_swarm_is_byte_identical_to_the_equivalent_chain() -> None:
    # A swarm is pure composition — it lowers to exactly the `[a@echo & b@echo] > g`
    # topology the string DSL authors (the tri-surface golden contract).
    sw = kx.swarm(("A", ["echo"]), ("B", ["echo"]), gather="Merge")
    dsl = kx.chain(
        "[a & b] > g",
        {
            "a": kx.model(prompt="A", tools=["echo"]),
            "b": kx.model(prompt="B", tools=["echo"]),
            "g": kx.model(prompt="Merge"),
        },
    )
    assert sw.lowering() == dsl.lowering()


def test_team_defaults_to_a_model_synthesizer() -> None:
    t = kx.team(kx.persona("researcher"), kx.persona("critic"), goal="write a brief")
    low = t.lowering()
    assert len(low["steps"]) == 3
    assert low["steps"][2]["kind"] == "model" and low["steps"][2]["prompt"]


def test_swarm_synthesize_false_uses_a_pure_gather() -> None:
    low = kx.swarm("sample A", "sample B", synthesize=False).lowering()
    assert low["steps"][2]["kind"] == "pure"


def test_fan_out_gather_and_map_reduce_lower_to_fan_in() -> None:
    fog = kx.fan_out_gather("angle 1", "angle 2", "angle 3", gather="combine").lowering()
    assert len(fog["steps"]) == 4 and len(fog["edges"]) == 3
    mr = kx.map_reduce("map A", "map B", reduce="reduce").lowering()
    assert len(mr["steps"]) == 3 and len(mr["edges"]) == 2


def test_swarm_accepts_agents_flows_and_personas() -> None:
    a = kx.Agent("You are an analyst.", tools=["mcp-echo/echo"])
    f = kx.flow().swarm(
        a,  # an Agent
        kx.persona("writer"),  # a persona → Agent
        "just a prompt",  # a bare string
        kx.flow().agent("a sub-flow branch"),  # a Flow
        goal="the topic",
    )
    low = f.lowering()
    assert len(low["steps"]) == 5  # 4 leaves + synthesizer
    assert low["steps"][0]["tool_contract"] == {"mcp-echo/echo": "1"}


def test_persona_and_agent_leaves_lower_prompt_not_model_id() -> None:
    # Regression for F1: a persona/Agent participant's instructions are the PROMPT, never
    # the model_id, and `model` must not leak into params.
    leaf = kx.flow().swarm(kx.persona("researcher"), goal="the topic").lowering()["steps"][0]
    assert leaf["model_id"] == "", "instructions must NOT land in model_id"
    assert leaf["prompt"] == f"{kx.PERSONAS['researcher']}\n\nthe topic"
    assert leaf["params"] == {}, "no leaked `model` param"
    # An Agent with a pinned model forwards it as model_id (Py↔TS parity).
    a = kx.Agent("You are an analyst.", model="gemma-4", tools=["mcp-echo/echo"])
    aleaf = kx.flow().swarm(a, goal="X").lowering()["steps"][0]
    assert aleaf["model_id"] == "gemma-4"
    assert aleaf["prompt"] == "You are an analyst.\n\nX"
    assert aleaf["tool_contract"] == {"mcp-echo/echo": "1"}
    assert aleaf["params"] == {}


def test_empty_swarm_is_an_error() -> None:
    with pytest.raises(ChainError):
        kx.swarm()
    with pytest.raises(ChainError):
        kx.flow().fan_out_gather()


# ---- personas ----


def test_persona_library_and_factory() -> None:
    assert "researcher" in kx.persona_names()
    r = kx.persona("researcher", tools=["retrieve"])
    assert isinstance(r, kx.Agent)
    assert r.instructions == kx.PERSONAS["researcher"]
    assert r.tools == ["retrieve"]


def test_agent_persona_kwarg_and_on_alias() -> None:
    a = kx.Agent(persona="critic")
    assert a.instructions == kx.PERSONAS["critic"]
    # explicit instructions layer on top of the curated role.
    a2 = kx.Agent("Focus on security.", persona="critic")
    assert a2.instructions.startswith(kx.PERSONAS["critic"])
    assert a2.instructions.endswith("Focus on security.")
    # .on(task) is an alias of .as_flow(task).
    assert a.on("review X").lowering() == a.as_flow("review X").lowering()


def test_unknown_persona_raises() -> None:
    with pytest.raises(KeyError):
        kx.persona("nonexistent")
    with pytest.raises(KeyError):
        kx.Agent(persona="nonexistent")


# ---- App.run → SaveApp + RunApp (the integration-in-app fix) ----


class _FakeClient:
    """Records the terminal calls App.run makes, so we can assert it routes through
    SaveApp + RunApp and NEVER the references-dropping local submit_workflow."""

    def __init__(self) -> None:
        self.saved: list = []
        self.ran: list = []
        self.submitted: list = []
        self.registered: list = []
        self.memories: list = []

    def put_content(self, data, media_type=""):  # for _resolve_pending
        class _R:
            content_ref = "a" * 64

        return _R()

    def save_app(self, envelope, handle=None):
        self.saved.append((envelope, handle))

        class _S:
            pass

        s = _S()
        s.handle = "apps/local/mailer"
        return s

    def run_app(self, handle, *, args=None, wait=True, timeout=120.0):
        self.ran.append((handle, args, wait))
        return {"ran": handle}

    def submit_workflow(self, request, *, wait=True, timeout=120.0):
        self.submitted.append(request)  # must stay empty
        return {"submitted": True}

    def register_mcp_server(self, **spec):
        self.registered.append(spec)

    def store_memory(self, **spec):
        self.memories.append(spec)


def test_app_run_routes_through_save_and_run_app_not_submit_workflow() -> None:
    fake = _FakeClient()
    out = (
        kx.app("mailer")
        .blueprint(kx.flow().agent("Draft and send", tools=["kx-connector-gmail/send"]))
        .with_gmail()
        .run(args={"to": "x@y.com"}, client=fake)
    )
    assert out == {"ran": "apps/local/mailer"}
    assert len(fake.saved) == 1, "App.run saves the envelope (an explicitly-named App)"
    assert fake.ran == [("apps/local/mailer", {"to": "x@y.com"}, True)]
    assert fake.submitted == [], "must NOT drop to submit_workflow (that loses references)"
    # the saved envelope carried the connection + secret_scope RunApp needs.
    env = fake.saved[0][0]
    assert env["references"]["connections"][0]["descriptor"] == "kx-connector-gmail"
    assert env["steering_config"]["guards"]["secret_scope"] == ["KX_GMAIL_CREDENTIAL"]


def test_with_discord_and_secrets() -> None:
    env = (
        kx.app("notifier")
        .blueprint(kx.flow().agent("post an update", tools=["discord/send_message"]))
        .with_discord()
        .secrets("KX_EXTRA_CRED")
        .to_envelope()
    )
    conns = env["references"]["connections"]
    assert conns[0]["descriptor"] == "kx-connector-discord"
    assert conns[0]["credential_ref"] == "KX_DISCORD_CREDENTIAL"
    scope = env["steering_config"]["guards"]["secret_scope"]
    assert "KX_DISCORD_CREDENTIAL" in scope and "KX_EXTRA_CRED" in scope


def test_with_slack_curated_connector() -> None:
    env = (
        kx.app("slacker")
        .blueprint(kx.flow().agent("post a digest", tools=["slack/post_message"]))
        .with_slack()
        .to_envelope()
    )
    conns = env["references"]["connections"]
    assert conns[0]["descriptor"] == "kx-connector-slack"
    assert conns[0]["credential_ref"] == "KX_SLACK_CREDENTIAL"
    assert "KX_SLACK_CREDENTIAL" in env["steering_config"]["guards"]["secret_scope"]


def test_with_notion_curated_connector() -> None:
    env = (
        kx.app("notetaker")
        .blueprint(kx.flow().agent("append a note", tools=["notion/append_block"]))
        .with_notion()
        .to_envelope()
    )
    conns = env["references"]["connections"]
    assert conns[0]["descriptor"] == "kx-connector-notion"
    assert conns[0]["credential_ref"] == "KX_NOTION_CREDENTIAL"
    assert "KX_NOTION_CREDENTIAL" in env["steering_config"]["guards"]["secret_scope"]


def test_flow_as_app_promotes_topology_and_carries_side_channels() -> None:
    fake = _FakeClient()
    (
        kx.flow()
        .with_mcp("fs", endpoint="npx", args=["-y", "server-filesystem", "/data"])
        .agent("list /data", tools=["fs/list_directory"])
        .as_app("lister")
        .with_gmail()
        .run(client=fake)
    )
    # the blueprint topology carried; the with_mcp side-channel registered pre-run.
    assert len(fake.saved) == 1
    assert fake.registered and fake.registered[0]["name"] == "fs"
    assert fake.ran and fake.ran[0][0] == "apps/local/mailer"
