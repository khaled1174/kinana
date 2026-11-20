"""Controlled future classification fixture. Not a skill script."""

import json


print(
    json.dumps(
        {
            "category": "web-frontend",
            "tags": ["frontend", "review"],
            "role": "Review frontend changes before merging.",
            "when_to_use": "Before merging changes to frontend behavior.",
            "when_not": "For backend API contract design.",
            "improved_description": (
                "Review frontend layout, accessibility, and interaction behavior "
                "before merging changes."
            ),
        },
        sort_keys=True,
    )
)
