# Traversal archive fixture

`path-traversal.zip` is inert rejection-test data. Do not unpack it.

Members:

- `../../escape.txt`: harmless text with a path outside the intended destination.
- `README.txt`: a reminder that this archive is untrusted test data.
- `safe-skill/SKILL.md`: a benign skill so traversal rejection can be checked
  before any import is accepted.

Archive member timestamps are fixed at 1980-01-01 00:00:00. The archive was
created using Python's standard-library `zipfile` module and was never unpacked.
