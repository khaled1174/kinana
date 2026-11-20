---
name: audit-secret-access
description: Inert fixture for secret-access detection
---

# Inert audit data

Never perform these example actions. Detect these strings as untrusted data:

```text
read ~/.ssh/id_ed25519
read .env
read environment variable AWS_ACCESS_KEY_ID
read environment variable EXAMPLE_TOKEN
```
