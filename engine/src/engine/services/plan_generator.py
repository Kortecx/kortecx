"""
Plan generator — builds execution DAGs from PRISM graph using semantic similarity.

Generates plans by:
1. Fetching expert nodes from Qdrant
2. Computing similarity edges (cosine distance)
3. Optionally using an LLM to select/order experts based on a user prompt
4. Building a DAG with topological ordering and parallel/sequential grouping
"""

from __future__ import annotations

import logging
from collections import defaultdict, deque
from typing import Any

from engine.services.local_inference import inference_router
from engine.services.qdrant import qdrant_service

logger = logging.getLogger("engine.services.plan_generator")

PRISM_COLLECTION = "kortecx_prisms"

PLAN_SYSTEM_PROMPT = """You are an AI workflow planner for the Kortecx platform.
Given a list of available AI experts (agents) and a user's goal, select the relevant
experts and determine the execution order. Output a JSON array of objects:
[{"expertId": "...", "label": "...", "dependsOn": ["expertId1", ...]}]
Only include experts that are relevant to the goal. Use dependsOn to express
which experts must complete before this one can start. Experts with empty
dependsOn arrays can run in parallel."""


async def fetch_prism_nodes() -> list[dict[str, Any]]:
    """Fetch all expert nodes from the Qdrant PRISM collection."""
    try:
        collections = qdrant_service.client.get_collections().collections
        if PRISM_COLLECTION not in [c.name for c in collections]:
            return []

        scroll_result = qdrant_service.client.scroll(
            collection_name=PRISM_COLLECTION,
            limit=500,
            with_vectors=False,
            with_payload=True,
        )
        points = scroll_result[0]
        return [
            {
                "id": p.payload.get("expert_id", str(p.id)),
                "name": p.payload.get("name", "Unknown"),
                "category": p.payload.get("category", ""),
                "description": p.payload.get("description", ""),
                "tags": p.payload.get("tags", []),
                "systemPrompt": p.payload.get("system_prompt", ""),
            }
            for p in points
        ]
    except Exception:
        logger.exception("Failed to fetch PRISM nodes")
        return []


async def fetch_prism_edges(threshold: float = 0.3, limit: int = 20) -> list[dict[str, Any]]:
    """Compute similarity edges from Qdrant. Reuses the same logic as experts.get_graph_edges."""
    try:
        collections = qdrant_service.client.get_collections().collections
        if PRISM_COLLECTION not in [c.name for c in collections]:
            return []

        scroll_result = qdrant_service.client.scroll(
            collection_name=PRISM_COLLECTION,
            limit=500,
            with_vectors=True,
            with_payload=True,
        )
        points = scroll_result[0]
        if len(points) < 2:
            return []

        edges: list[dict[str, Any]] = []
        seen: set[str] = set()

        for point in points:
            expert_id = point.payload.get("expert_id", str(point.id))
            results = qdrant_service.client.query_points(
                collection_name=PRISM_COLLECTION,
                query=point.vector,
                limit=limit + 1,
                score_threshold=threshold,
            )
            for hit in results.points:
                target_id = hit.payload.get("expert_id", str(hit.id))
                if target_id == expert_id:
                    continue
                edge_key = tuple(sorted([expert_id, target_id]))
                key_str = f"{edge_key[0]}:{edge_key[1]}"
                if key_str in seen:
                    continue
                seen.add(key_str)
                edges.append({
                    "source": expert_id,
                    "target": target_id,
                    "weight": round(hit.score, 4),
                })

        edges.sort(key=lambda e: e["weight"], reverse=True)
        return edges
    except Exception:
        logger.exception("Failed to fetch PRISM edges")
        return []


def _topological_sort(
    node_ids: list[str],
    dependencies: dict[str, list[str]],
) -> list[list[str]]:
    """Topological sort returning layers of parallelizable nodes.

    Each layer contains nodes whose dependencies are all in prior layers.
    """
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
    """Convert topological layers into PlanNode[] and PlanEdge[] for the DAG."""
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


async def generate_plan_from_graph(
    workflow_id: str | None = None,
    prompt: str | None = None,
    model: str = "llama3.1:8b",
    engine: str = "ollama",
) -> dict[str, Any]:
    """Generate a plan DAG from the PRISM graph.

    If prompt is provided, uses the LLM to select and order experts.
    Otherwise, builds a DAG from all experts using similarity edges.
    """
    experts = await fetch_prism_nodes()
    if not experts:
        return {"nodes": [], "edges": [], "error": "No PRISM experts found"}

    expert_map = {e["id"]: e for e in experts}

    if prompt:
        return await _generate_with_llm(expert_map, prompt, model, engine)
    else:
        return await _generate_from_similarity(expert_map)


async def _generate_with_llm(
    expert_map: dict[str, dict[str, Any]],
    prompt: str,
    model: str,
    engine: str,
) -> dict[str, Any]:
    """Use LLM to select relevant experts and determine ordering."""
    expert_summaries = "\n".join(
        f"- {e['id']}: {e['name']} ({e['category']}) — {e['description'][:100]}"
        for e in expert_map.values()
    )

    user_prompt = f"""Available experts:
{expert_summaries}

User's goal: {prompt}

Select the relevant experts and output the execution plan as a JSON array."""

    try:
        result = await inference_router.generate(
            engine=engine,
            model=model,
            prompt=user_prompt,
            system=PLAN_SYSTEM_PROMPT,
            temperature=0.3,
            max_tokens=2048,
        )

        import json
        # Try to parse JSON from the response
        text = result.text.strip()
        # Find JSON array in response
        start = text.find("[")
        end = text.rfind("]") + 1
        if start >= 0 and end > start:
            plan_items = json.loads(text[start:end])
        else:
            logger.warning("LLM did not return valid JSON, falling back to similarity")
            return await _generate_from_similarity(expert_map)

        # Build dependencies map
        dependencies: dict[str, list[str]] = {}
        node_ids: list[str] = []

        for item in plan_items:
            eid = item.get("expertId", "")
            if eid in expert_map:
                node_ids.append(eid)
                deps = [d for d in item.get("dependsOn", []) if d in expert_map]
                dependencies[eid] = deps

        if not node_ids:
            return await _generate_from_similarity(expert_map)

        layers = _topological_sort(node_ids, dependencies)
        dag = _build_dag_from_layers(layers, expert_map, dependencies)
        dag["generatedBy"] = "ai"
        dag["modelUsed"] = model
        return dag

    except Exception:
        logger.exception("LLM plan generation failed, falling back to similarity")
        return await _generate_from_similarity(expert_map)


async def _generate_from_similarity(
    expert_map: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    """Build a DAG from all experts using similarity edges.

    High-similarity experts with bidirectional edges run in parallel.
    Lower-similarity connections form sequential chains.
    """
    edges = await fetch_prism_edges(threshold=0.3, limit=20)
    node_ids = list(expert_map.keys())

    # Build adjacency: experts with strong similarity (>0.6) are independent (parallel).
    # For weaker edges, treat source→target as a dependency (sequential).
    dependencies: dict[str, list[str]] = {nid: [] for nid in node_ids}

    # Sort edges by weight descending; higher weight = more similar = parallel candidates
    high_threshold = 0.6
    for edge in edges:
        src, tgt = edge["source"], edge["target"]
        if src not in expert_map or tgt not in expert_map:
            continue
        if edge["weight"] < high_threshold:
            # Lower similarity: sequential dependency (source runs before target)
            if src in dependencies and tgt in dependencies:
                dependencies[tgt].append(src)

    layers = _topological_sort(node_ids, dependencies)
    dag = _build_dag_from_layers(layers, expert_map, dependencies)
    dag["generatedBy"] = "prism_graph"
    return dag
