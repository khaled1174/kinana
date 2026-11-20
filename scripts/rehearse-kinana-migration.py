#!/usr/bin/env python3
"""Rehearse on a complete COPY of the existing UI vault, never its original.

Seed legacy state/router/link with the saved old executable. Keep the rehearsal
and evidence for review; no provider calls, global installation, or real migration.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--evidence", default="docs/evidence/kinana-rename/rehearsal")
E = REPO / parser.parse_args().evidence
E.mkdir()
fixture = json.loads((REPO / ".git/phase62-live-fixture.json").read_text())
ORIGINAL = Path(fixture["vault"])
OLD = REPO / ".git/kinana-rename-baseline/skillsbox"
NEW = REPO / "target/debug/kinana"
BASE = Path(tempfile.mkdtemp(prefix="kinana-migration-rehearsal-")).resolve()
HOME = BASE / "home"
VAULT = BASE / "vault-copy"
PROJECT = BASE / "مشروع تجربة مربوط"
HOME.mkdir(); PROJECT.mkdir()
ENV = {**os.environ, "HOME": str(HOME), "USERPROFILE": str(HOME),
       "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"}
commands = []


def save(name, value):
    (E / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def tree(root):
    result = {}
    for at, dirs, files in os.walk(root, followlinks=False):
        for name in sorted(dirs + files):
            path = Path(at) / name
            metadata = path.lstat()
            item = {"mode": stat.S_IMODE(metadata.st_mode),
                    "identity": f"{metadata.st_dev}:{metadata.st_ino}"}
            if path.is_symlink():
                item.update(kind="symlink", target=os.readlink(path))
            elif path.is_dir(): item["kind"] = "directory"
            else:
                digest = hashlib.sha256()
                with path.open("rb") as stream:
                    for part in iter(lambda: stream.read(1024 * 1024), b""): digest.update(part)
                item.update(kind="file", sha256=digest.hexdigest())
            result[str(path.relative_to(root))] = item
    return result


def record(label, argv, expect=0, cwd=BASE):
    argv = [str(value) for value in argv]
    command = {"label": label, "argv": argv, "cwd": str(cwd),
               "environment_overrides": {key: ENV[key] for key in ("HOME", "USERPROFILE", "GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM")}}
    save(label + ".command.json", command)
    result = subprocess.run(argv, cwd=cwd, env=ENV, text=True, capture_output=True, timeout=120)
    for suffix, value in (("stdout.txt", result.stdout), ("stderr.txt", result.stderr), ("exit.txt", str(result.returncode) + "\n")):
        (E / (label + "." + suffix)).write_text(value)
    commands.append({**command, "exit": result.returncode})
    save("commands.json", commands)
    print(label + ": exit " + str(result.returncode), flush=True)
    assert result.returncode == expect, (label, result.stdout, result.stderr)
    return result.stdout


def box(label, binary, *args, expect=0):
    return json.loads(record(label, [binary, "--vault", VAULT, "--json", *args], expect))


assert ORIGINAL.is_dir() and OLD.is_file() and NEW.is_file()
assert not ORIGINAL.is_relative_to(BASE) and not BASE.is_relative_to(ORIGINAL)
original_before = tree(ORIGINAL)
save("original-vault-before.json", original_before)
save("state.json", {"base": str(BASE), "home": str(HOME), "vault_copy": str(VAULT),
                   "original_vault": str(ORIGINAL), "project": str(PROJECT),
                   "old_binary": str(OLD), "new_binary": str(NEW),
                   "old_binary_sha256": hashlib.sha256(OLD.read_bytes()).hexdigest(),
                   "new_binary_sha256": hashlib.sha256(NEW.read_bytes()).hexdigest(),
                   "real_home_legacy_state_existed": (Path.home() / ".skillsbox").exists(),
                   "real_home_current_state_existed": (Path.home() / ".kinana").exists()})
shutil.copytree(ORIGINAL, VAULT, symlinks=True)
box("old-init-router", OLD, "init", "--install-router")
box("old-rebuild-index", OLD, "doctor", "--rebuild-index")
record("project-git-init", ["/usr/bin/git", "-c", "core.hooksPath=/dev/null", "init", "-q", "--template=", PROJECT])
(PROJECT / "README عربي.md").write_text("Preserve this file, its mode, and the Git tree.\n")
(PROJECT / "README عربي.md").chmod(0o640)
initial_project = tree(PROJECT)
save("project-before-legacy-use.json", initial_project)
box("old-use-link", OLD, "use", "emil-color", "--project", PROJECT, "--agent", "codex", "--mode", "link")
target = PROJECT / ".agents/skills/emil-color"
assert target.is_symlink()
# Add only controlled future state/lock extensions to the copied fixture.
lock = PROJECT / ".skillsbox.lock"
raw = lock.read_text()
assert "[[active]]\n" in raw
lock.write_text('future_extension = "preserve-top-level"\n' + raw.replace("[[active]]\n", '[[active]]\nfuture_member = "preserve-member"\n', 1))
(HOME / ".skillsbox/إعدادات مستقبلية.bin").write_bytes(b"unknown-state\0\xff")
(HOME / ".skillsbox/إعدادات مستقبلية.bin").chmod(0o640)
# Keep committed WAL bytes as if an old process terminated without checkpointing.
wal_script = ("import os,sqlite3,sys\n"
              "c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA journal_mode=WAL');c.execute('PRAGMA wal_autocheckpoint=0');"
              "c.execute('CREATE TABLE kinana_rehearsal_probe(value TEXT)');"
              "c.execute(\"INSERT INTO kinana_rehearsal_probe VALUES('committed WAL survives')\");c.commit();os._exit(0)\n")
record("seed-committed-wal", [sys.executable, "-c", wal_script, HOME / ".skillsbox/index.db"])
assert (HOME / ".skillsbox/index.db-wal").is_file()
before_project, before_state, before_vault = tree(PROJECT), tree(HOME / ".skillsbox"), tree(VAULT)
save("project-before-migration.json", before_project)
save("legacy-state-before.json", before_state)
save("vault-copy-before-migration.json", before_vault)
all_before = tree(BASE)
plan = box("migration-preview", NEW, "migrate", "--dry-run")
assert tree(BASE) == all_before
assert len([p for p in plan["projects"] if p["state"] == "rename"]) == 1
assert len(plan["routers"]) == 2
box("migration-reject-stale", NEW, "migrate", "--confirm", "invalid-confirmation", expect=2)
assert tree(BASE) == all_before
result = box("migration-confirm", NEW, "migrate", "--confirm", plan["preview_token"])
after_project = tree(PROJECT)
expected_project = {(".kinana.lock" if key == ".skillsbox.lock" else key): value for key, value in before_project.items()}
assert after_project == expected_project
assert tree(HOME / ".skillsbox") == before_state
assert tree(VAULT) == before_vault
save("project-after-migration.json", after_project)
db = sqlite3.connect("file:" + str(HOME / ".kinana/index.db") + "?mode=ro", uri=True)
assert db.execute("SELECT value FROM kinana_rehearsal_probe").fetchone()[0] == "committed WAL survives"
assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
db.close()
assert box("status-after-migration", NEW, "status", "--project", PROJECT)["projects"][0]["items"][0]["state"] == "current"
assert box("repeat-preview", NEW, "migrate", "--dry-run")["changed"] is False
rollback = box("rollback-preview", NEW, "migrate", "--rollback", "--dry-run")
box("rollback-confirm", NEW, "migrate", "--rollback", "--confirm", rollback["preview_token"])
assert tree(PROJECT) == before_project
assert tree(HOME / ".skillsbox") == before_state
assert not (HOME / ".kinana").exists()
save("project-after-rollback.json", tree(PROJECT))
plan = box("reapply-preview", NEW, "migrate", "--dry-run")
box("reapply-confirm", NEW, "migrate", "--confirm", plan["preview_token"])
box("new-unuse", NEW, "unuse", "emil-color", "--project", PROJECT, "--agent", "codex")
assert tree(PROJECT) == initial_project
assert tree(VAULT) == before_vault
assert tree(ORIGINAL) == original_before
save("project-after-unuse.json", tree(PROJECT))
save("original-vault-after.json", tree(ORIGINAL))
fingerprint = lambda value: hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False).encode()).hexdigest()
report = {"source": "complete copy of the existing real emil skills vault used by the UI",
          "base": str(BASE), "retained": True, "real_user_state_migrated": False,
          "original_vault_unchanged": True, "copied_vault_unchanged_by_migration_or_unuse": True,
          "legacy_data_seeded_by_actual_old_binary": True, "preview_and_invalid_confirmation_write_free": True,
          "registered_project_found_without_home_scan": True, "owned_routers_migrated": 2,
          "lock_bytes_permissions_and_inode_preserved": True, "link_target_and_inode_preserved": True,
          "legacy_state_unchanged": True, "unknown_state_and_lock_fields_preserved": True,
          "sqlite_wal_commits_preserved": True, "sqlite_integrity_check": "ok",
          "rollback_restored_project_exactly": True, "reapply_succeeded": True,
          "unuse_restored_pre_activation_tree_hash_types_modes_and_identities": True,
          "project_before_use_sha256": fingerprint(initial_project),
          "project_after_unuse_sha256": fingerprint(tree(PROJECT)),
          "recovery_journal": result["journal"]}
save("verification.json", report)
print(json.dumps(report, ensure_ascii=False, indent=2), flush=True)
