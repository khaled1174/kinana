---
name: audit-escaping-symlink
description: Inert fixture for an escaping symbolic link
---

# Inert audit data

`outside.txt` is a symbolic link to `../symlink-target.txt`. The target is
outside this skill directory. Inspect the link itself without following it.
