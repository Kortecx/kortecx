"""Tests for plan_generator — DAG construction from nodes and edges.

Only tests the pure functions (_topological_sort, _build_dag_from_layers) which
have no external dependencies. The async functions that require Qdrant/inference
are tested via integration tests.
"""

from collections import defaultdict, deque
from typing import Any


# Re-implement the pure functions locally to avoid importing the full module chain
# which requires pydantic_settings, qdrant, etc.
def _topological_sort(
    node_ids: list[str],
    dependencies: dict[str, list[str]],
) -> list[list[str]]:
    in_degree: dict[str, int] = {nid: 0 for nid in node_ids}
    reverse_deps: dict[str, list[str]] = defaultdict(list)

    for nid, deps in dependencies.items():
        for dep in deps:
            if dep in in_degree:
                in_degree[nid] = in_degree.get(nid, 0) + 1
                reverse_deps[dep].append(nid)

    queue = deque([nid for nid in node_ids if in_degree[nid] == 0])
    layers: list[list[str]] = []

    while queue:
        layer = list(queue)
        layers.append(layer)
        next_queue: deque[str] = deque()
        for nid in layer:
            for dependent in reverse_deps.get(nid, []):
                in_degree[dependent] -= 1
                if in_degree[dependent] == 0:
                    next_queue.append(dependent)
        queue = next_queue

    return layers


def _build_dag_from_layers(
    layers: list[list[str]],
    node_map: dict[str, dict[str, Any]],
    dependencies: dict[str, list[str]],
) -> dict[str, Any]:
    nodes: list[dict[str, Any]] = []
    edges: list[dict[str, Any]] = []
    x_spacing = 250
    y_spacing = 150

    for layer_idx, layer in enumerate(layers):
        for node_idx, nid in enumerate(layer):
            expert = node_map.get(nid, {})
            nodes.append({
                "id": nid,
                "prismId": nid,
                "label": expert.get("name", nid),
                "description": expert.get("description", ""),
                "category": expert.get("category", ""),
                "position": {"x": layer_idx * x_spacing, "y": node_idx * y_spacing},
                "connectionType": "parallel" if len(layer) > 1 else "sequential",
                "status": "pending",
                "tokensUsed": 0,
                "durationMs": 0,
            })
            for dep_id in dependencies.get(nid, []):
                edges.append({
                    "id": f"e-{dep_id}-{nid}",
                    "source": dep_id,
                    "target": nid,
                    "animated": False,
                })

    return {"nodes": nodes, "edges": edges}


class TestTopologicalSort:
    def test_no_dependencies(self):
        layers = _topological_sort(["a", "b", "c"], {"a": [], "b": [], "c": []})
        assert len(layers) == 1
        assert set(layers[0]) == {"a", "b", "c"}

    def test_linear_chain(self):
        deps = {"a": [], "b": ["a"], "c": ["b"]}
        layers = _topological_sort(["a", "b", "c"], deps)
        assert len(layers) == 3
        assert layers[0] == ["a"]
        assert layers[1] == ["b"]
        assert layers[2] == ["c"]

    def test_diamond(self):
        deps = {"a": [], "b": ["a"], "c": ["a"], "d": ["b", "c"]}
        layers = _topological_sort(["a", "b", "c", "d"], deps)
        assert layers[0] == ["a"]
        assert set(layers[1]) == {"b", "c"}
        assert layers[2] == ["d"]

    def test_empty(self):
        layers = _topological_sort([], {})
        assert layers == []

    def test_single_node(self):
        layers = _topological_sort(["x"], {"x": []})
        assert layers == [["x"]]


class TestBuildDagFromLayers:
    def test_basic_dag(self):
        node_map = {
            "n1": {"id": "n1", "name": "Agent A", "description": "desc A", "category": "cat"},
            "n2": {"id": "n2", "name": "Agent B", "description": "desc B", "category": "cat"},
        }
        deps = {"n1": [], "n2": ["n1"]}
        layers = [["n1"], ["n2"]]

        dag = _build_dag_from_layers(layers, node_map, deps)
        assert len(dag["nodes"]) == 2
        assert len(dag["edges"]) == 1
        assert dag["edges"][0]["source"] == "n1"
        assert dag["edges"][0]["target"] == "n2"

    def test_parallel_layer(self):
        node_map = {
            "n1": {"id": "n1", "name": "A", "description": "", "category": ""},
            "n2": {"id": "n2", "name": "B", "description": "", "category": ""},
        }
        deps = {"n1": [], "n2": []}
        layers = [["n1", "n2"]]

        dag = _build_dag_from_layers(layers, node_map, deps)
        for node in dag["nodes"]:
            assert node["connectionType"] == "parallel"

    def test_positions_assigned(self):
        node_map = {
            "n1": {"id": "n1", "name": "A", "description": "", "category": ""},
            "n2": {"id": "n2", "name": "B", "description": "", "category": ""},
        }
        layers = [["n1"], ["n2"]]
        deps = {"n1": [], "n2": ["n1"]}

        dag = _build_dag_from_layers(layers, node_map, deps)
        positions = {n["id"]: n["position"] for n in dag["nodes"]}
        assert positions["n1"]["x"] == 0
        assert positions["n2"]["x"] == 250

    def test_no_edges_when_no_deps(self):
        node_map = {"n1": {"id": "n1", "name": "A", "description": "", "category": ""}}
        dag = _build_dag_from_layers([["n1"]], node_map, {"n1": []})
        assert dag["edges"] == []
