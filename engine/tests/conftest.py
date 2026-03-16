"""Shared test fixtures."""
import pytest


@pytest.fixture
def sample_step_config():
    """A minimal step configuration for orchestrator tests."""
    return {
        "step_id": "step-001",
        "expert_id": None,
        "task_description": "Generate a summary",
        "model_source": "local",
        "local_model": {"engine": "ollama", "model": "llama3.1:8b"},
        "temperature": 0.7,
        "max_tokens": 1024,
        "connection_type": "sequential",
    }


@pytest.fixture
def sample_workflow_request():
    """A minimal workflow request."""
    return {
        "workflow_id": "wf-test-001",
        "name": "Test Workflow",
        "goal_file_url": "test-goal.md",
        "input_file_urls": [],
        "steps": [],
    }


@pytest.fixture
def sample_synthesis_config():
    """A minimal synthesis config."""
    return {
        "name": "Test Dataset",
        "description": "Test data generation",
        "source": "ollama",
        "model": "llama3.1:8b",
        "targetSamples": 10,
        "outputFormat": "jsonl",
        "temperature": 0.8,
        "maxTokens": 512,
        "batchSize": 2,
    }
