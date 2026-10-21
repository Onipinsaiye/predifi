#!/usr/bin/env python3
"""Print resource actions from a Terraform plan without rendering attributes."""

import json
import sys


plan = json.load(sys.stdin)
changes = []
for resource in plan.get("resource_changes", []):
    actions = resource.get("change", {}).get("actions", [])
    if actions != ["no-op"]:
        changes.append((resource.get("address", "unknown"), ", ".join(actions)))

counts = {"create": 0, "update": 0, "delete": 0, "replace": 0}
for _, action_text in changes:
    actions = action_text.split(", ")
    if "create" in actions and "delete" in actions:
        counts["replace"] += 1
    else:
        for action in actions:
            if action in counts:
                counts[action] += 1

print("## Staging Terraform plan")
print()
print(
    "| Create | Update | Delete | Replace |\n|---:|---:|---:|---:|\n"
    f"| {counts['create']} | {counts['update']} | {counts['delete']} | {counts['replace']} |"
)
print()
if changes:
    print("| Resource | Actions |\n|---|---|")
    for address, actions in changes:
        print(f"| `{address}` | `{actions}` |")
else:
    print("No resource changes.")
print()
print("Sensitive values and resource attributes are intentionally omitted.")