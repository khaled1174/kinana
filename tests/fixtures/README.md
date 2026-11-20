# Inert test fixtures

These files are test data for `docs/SPEC.md` sections 8 and 9. Treat every
`SKILL.md`, code sample, script, symlink, and archive as untrusted data. Never
execute the skill files or unpack the traversal archive by hand.

## Phase 1 scan set

Scan **only `scan/projects/`**, against an empty vault, for these exact counts:

| State | Count |
| --- | ---: |
| new | 2 |
| duplicate | 1 |
| variant | 1 |
| broken | 2 |
| total candidates | 6 |

The valid skills have two names: `frontend-review` and `api-contract`. The
`frontend-review` files in project A and project B are byte-for-byte identical;
project C uses the same name with a different description and body. Which
identical copy is reported as new is an ordering choice. The totals above do
not depend on filesystem iteration order when the batch groups exact hashes
before comparing same-name variants. A sorted path traversal selects project
A's `frontend-review` as new, project B's as duplicate, and project C's as
variant.

| Relative to `scan/projects/` | Relationship |
| --- | --- |
| `project-a/.claude/skills/frontend-review/` | canonical identical copy |
| `project-b/.agents/skills/frontend-review/` | exact duplicate |
| `project-c/.codex/skills/frontend-review/` | same name, different content |
| `project-a/.agents/skills/api-contract/` | second unique name |
| `project-b/.claude/skills/missing-description/` | missing required description |
| `project-c/.agents/skills/malformed/` | invalid YAML frontmatter |

Three additional `SKILL.md` decoys appear below `node_modules/`, `.git/`, and
`plugins/`. They must be ignored and must not affect the six-candidate totals.
The `.git` decoy is an ordinary directory of inert fixture data, not an actual
repository. Git itself excludes directories named `.git` from normal tracking;
test setup must preserve or recreate that decoy when copying from a checkout.

## Future audit and import fixtures

`audit/` and `archives/` are outside the phase 1 scan root. They prepare the
later security tests without implementing import or audit behavior.

| Fixture under `audit/` | Section 8 row | Expected level from section 8 |
| --- | --- | --- |
| `script-file/` | script file present | review |
| `external-network/` | external connection strings | review |
| `secret-access/` | secret and environment access strings | risky |
| `destructive-command/` | destructive command strings | risky |
| `download-execute/` | download piped into a shell | risky |
| `frontmatter-hooks/` | hooks in frontmatter | review |
| `injection-ar/` | Arabic instruction override and concealment | risky |
| `injection-en/` | English instruction override and concealment | risky |
| `escaping-symlink/` | symlink leaves the skill directory | risky |

The examples are inert text. `script-file/scripts/probe.sh` contains comments
only, has no executable bit, and must never be run. The symlink
`audit/escaping-symlink/outside.txt` points to `../symlink-target.txt`, a harmless
sentinel within these fixtures but outside that skill directory. Do not follow
it when scanning or importing.

`archives/path-traversal.zip` contains the member `../../escape.txt`, an inert
sentinel. It also contains `README.txt` and a benign `safe-skill/SKILL.md` member.
The archive is data for rejection tests. Do not unpack it. Its fixed member
timestamps make the fixture reproducible.

Section 8 assigns `review` to script, network, and hook findings; that is what
the table above records. The broader phase 2 wording about malicious fixtures
does not change those row-specific levels in this fixture catalog.

## Future controlled classifier fixture

`agents/fake-agent.py` is a controlled test agent that returns one fixed JSON
object. It reads no skill files and makes no network requests. The JSON contains
classification fields plus an improved-description proposal; the phase 3
implementation must define and validate its exact schema before using it.

The script is intentionally stored without executable permission. It was not
executed or passed through `chmod` during fixture creation. Only a future,
explicitly scoped classification test may invoke the controlled agent; skill
scripts remain data and must never be executed. No live agent or macOS behavior
is exercised by these fixtures.
