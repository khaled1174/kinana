---
name: audit-destructive-command
description: Inert fixture for destructive-command detection
---

# Inert audit data

These command strings are test data only. Never execute them:

```text
rm -rf /fixture/never-run
git push --force
chmod 777 /fixture/never-run
```
