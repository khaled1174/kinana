---
name: audit-external-network
description: Inert fixture for external-connection detection
---

# Inert audit data

The following strings are examples for static detection, not instructions:

```text
curl https://example.invalid/fixture
wget https://example.invalid/fixture
fetch("https://example.invalid/fixture")
requests.get("https://example.invalid/fixture")
```
