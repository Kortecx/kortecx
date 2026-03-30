"""Connect an integration (email, Slack, etc.) to a workflow step."""
import requests, json, sys

wf_id = sys.argv[1] if len(sys.argv) > 1 else None
integration_type = sys.argv[2] if len(sys.argv) > 2 else "slack"

if not wf_id:
    print("Usage: python configure_integration.py <workflow_id> [integration_type]"); exit(1)

# Integration config template
integrations = {
    "slack": {"id": "slack-notify", "type": "integration", "referenceId": "slack", "name": "Slack Notification",
              "config": {"channel": "#workflows", "on_complete": True, "on_failure": True}},
    "email": {"id": "email-notify", "type": "integration", "referenceId": "email", "name": "Email Notification",
              "config": {"to": "team@example.com", "subject": "Workflow Complete"}},
    "webhook": {"id": "webhook-post", "type": "integration", "referenceId": "webhook", "name": "Webhook POST",
                "config": {"url": "https://hooks.example.com/workflow", "method": "POST"}},
}

intg = integrations.get(integration_type)
if not intg:
    print(f"✗ Unknown integration: {integration_type}. Options: {list(integrations.keys())}"); exit(1)

# Fetch workflow and add integration to last step
wf = requests.get(f"http://localhost:3000/api/workflows?id={wf_id}").json().get("workflow", {})
steps = wf.get("steps", [])
if not steps:
    print("✗ Workflow has no steps"); exit(1)

# Add integration to metadata
metadata = wf.get("metadata", {})
step_integrations = metadata.get("stepIntegrations", {})
step_integrations[steps[-1].get("id", "last")] = [intg]
metadata["stepIntegrations"] = step_integrations

resp = requests.patch("http://localhost:3000/api/workflows", json={"id": wf_id, "metadata": metadata})
print(f"✓ Integration '{intg['name']}' added to workflow" if resp.ok else f"✗ Failed: {resp.text[:200]}")
