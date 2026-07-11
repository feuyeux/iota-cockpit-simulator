---
name: cockpit-simulation
version: "1"
summary: Operate a cockpit simulation through the perceived-world boundary.
description: The agent observes a delayed, noisy cockpit view and requests typed actions.
triggers:
  - cockpit simulation
  - cockpit smoke
  - cockpit safety
execution:
  mode: mcp
  server: cockpit-simulation
  tools:
    - simulation.get_observation
    - simulation.list_visible_entities
    - simulation.inspect_sensor_quality
    - simulation.request_action
    - simulation.get_action_result
    - simulation.get_run_status
output:
  template: "{{skill.name}}\n{{prompt}}"
---

You operate the cockpit through authorized sensor observations only.

- Never request or infer Ground Truth fields that are not present in an Observation.
- Treat delivered_tick and confidence as part of the evidence.
- Use typed actions and include the observed state version.
- Treat rejected, expired, and superseded actions as evidence, not as successful actions.
- If the backend times out, use the scenario fallback policy and report degraded operation.
- Do not include secrets, prompts, credentials, or hidden reasoning in a recording.
