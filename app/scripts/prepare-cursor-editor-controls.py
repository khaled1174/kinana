#!/usr/bin/env python3
"""Create two retained editor controls without changing the existing link probe.

A uses the same Kinana import/review/use flow, with a real copy. B writes
the same inert skill directly to .cursor/skills and never calls Kinana.
Markers and evidence stay outside the project except in skill descriptions.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import secrets
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SUPPORT = runpy.run_path(str(Path(__file__).with_name("prepare-cursor-editor-test.py")))
NAME, QUESTION, tree = (SUPPORT[key] for key in ("NAME", "QUESTION", "tree"))
INVOKE_QUESTION = f"Use the {NAME} skill and return its EDITOR_PROBE value."


def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")


def prepare(base, old, original, old_dist, old_marker, kind):
    base.mkdir(mode=0o700)
    base = base.resolve(strict=True)
    project = base / "project"
    project.mkdir()
    commands = []
    home = base / "home"
    home.mkdir()
    env = {**os.environ, "HOME": str(home), "USERPROFILE": str(home)}

    def run(argv):
        argv = [str(value) for value in argv]
        completed = subprocess.run(argv, env=env, text=True, capture_output=True, timeout=60)
        commands.append({"argv": argv, "cwd": str(ROOT),
                         "env": {key: env[key] for key in ("HOME", "USERPROFILE")},
                         "exit": completed.returncode, "stdout": completed.stdout,
                         "stderr": completed.stderr})
        save(base / "commands.json", commands)
        if completed.returncode:
            raise RuntimeError(commands[-1])
        return completed.stdout

    def box(*arguments):
        return json.loads(run([ROOT / "target/debug/kinana", "--vault", base / "vault",
                              "--json", *arguments]))

    run(["/usr/bin/git", "-c", "core.hooksPath=/dev/null", "init", "-q", "--template=", project])
    (project / "README.md").write_bytes((old / "project/README.md").read_bytes())
    save(base / "before-use-tree.json", tree(project))
    marker = "CURSOR_EDITOR_" + secrets.token_hex(16).upper()
    while marker == old_marker:
        marker = "CURSOR_EDITOR_" + secrets.token_hex(16).upper()
    old_expected = json.loads((old / "expected.json").read_text())
    description = old_expected["description_approved"].replace(old_marker, marker)
    expected_skill = old_dist.replace(old_marker.encode(), marker.encode())
    report = {"project": str(project), "skill": NAME, "case": kind,
              "question_1": QUESTION, "question_2": INVOKE_QUESTION,
              "retained_until_owner_results": True, "step_2_executed": False}

    if kind == "agents-copy":
        source, vault = base / "source", base / "vault"
        source.mkdir()
        (source / "SKILL.md").write_bytes(original)
        box("init")
        box("add", source)
        proposal = {"category": "other", "tags": ["probe"], "role": "Review native spacing",
                    "when_to_use": "A manual editor loading check", "when_not": "Other tasks",
                    "improved_description": description, "overlaps_with": [], "confidence": 1.0}
        metadata = vault / "meta" / (NAME + ".toml")
        metadata.write_text(metadata.read_text() + "\n[proposal]\n" + "".join(
            f"{key} = {json.dumps(value, ensure_ascii=False)}\n" for key, value in proposal.items()))
        for arguments in (
            ["add", "--", "meta/" + NAME + ".toml"],
            ["-c", "user.name=Kinana Test", "-c", "user.email=test@localhost",
             "commit", "-q", "-m", "Prepare controlled editor copy proposal"],
        ):
            run(["/usr/bin/git", "-C", vault, "-c", "core.hooksPath=/dev/null", *arguments])
        box("review", NAME, "--accept")
        assert (vault / "skills" / NAME / "SKILL.md").read_bytes() == original
        assert (vault / "dist" / NAME / "SKILL.md").read_bytes() == expected_skill
        # Keep the initial probe's logical agent; only activation mode changes.
        activation_agent = json.loads((old / "prepared.json").read_text())["activation_agent"]
        used = box("use", NAME, "--project", project, "--agent", activation_agent, "--mode", "copy")
        status = box("status", "--project", project)
        assert status["projects"][0]["items"][0]["state"] == "current"
        target = project / ".agents/skills" / NAME
        assert not (project / ".cursor").exists()
        report.update(vault=str(vault), activation_agent=activation_agent, mode="copy",
                      use=used, status=status, original_unchanged=True)
    else:
        # Direct filesystem fixture: no add, review, use, vault, or lock file.
        target = project / ".cursor/skills" / NAME
        target.mkdir(parents=True)
        (target / "SKILL.md").write_bytes(expected_skill)
        assert not (project / ".agents").exists()
        assert not (project / ".kinana.lock").exists()
        assert not (base / "vault").exists()
        assert len(commands) == 1 and commands[0]["argv"][0] == "/usr/bin/git"
        report.update(mode="direct", skillsbox_calls=0)

    assert target.is_dir() and not target.is_symlink()
    for entry in (target, target.parent, target.parent.parent):
        assert not entry.is_symlink()
    assert sorted(path.name for path in target.iterdir()) == ["SKILL.md"]
    actual = (target / "SKILL.md").read_bytes()
    assert actual == expected_skill
    frontmatter, body = actual.decode().split("---\n", 2)[1:]
    assert marker in frontmatter and actual.count(marker.encode()) == 1
    assert marker not in body and marker not in original.decode()
    assert marker not in QUESTION and marker not in INVOKE_QUESTION
    assert not (project / ".claude").exists() and not (project / ".codex").exists()
    assert not any(value["kind"] == "symlink" for value in tree(project).values())
    (base / "question-1.txt").write_text(QUESTION + "\n")
    (base / "question-2.txt").write_text(INVOKE_QUESTION + "\n")
    save(base / "expected.json", {"marker": marker,
         "description_original": old_expected["description_original"],
         "description_approved": description})
    report.update(target=str(target), real_directory=True, skill_files=["SKILL.md"],
                  marker_only_in_description=True,
                  content_equal_to_link_probe_except_marker=True,
                  skill_sha256=hashlib.sha256(actual).hexdigest())
    save(base / "after-use-tree.json", tree(project))
    save(base / "prepared.json", report)
    return report, marker


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--previous-directory", required=True, type=Path)
    parser.add_argument("--copy-directory", required=True, type=Path)
    parser.add_argument("--native-directory", required=True, type=Path)
    args = parser.parse_args()
    old, copy, native = (path.expanduser().resolve() for path in (
        args.previous_directory, args.copy_directory, args.native_directory))
    for index, path in enumerate((old, copy, native)):
        for other in (old, copy, native)[index + 1:]:
            if path == other or path in other.parents or other in path.parents:
                parser.error("Fixture directories must be separate and non-overlapping")
    if any(path.exists() or path.is_symlink() for path in (copy, native)):
        parser.error("Control directory already exists; refusing to change or reuse it")
    original_tree = tree(old)
    original = (old / "source/SKILL.md").read_bytes()
    assert (old / "vault/skills" / NAME / "SKILL.md").read_bytes() == original
    old_dist = (old / "vault/dist" / NAME / "SKILL.md").read_bytes()
    old_marker = json.loads((old / "expected.json").read_text())["marker"]
    assert old_marker.encode() not in original
    reports = []
    markers = [old_marker]
    try:
        for destination, kind in ((copy, "agents-copy"), (native, "cursor-direct")):
            report, marker = prepare(destination, old, original, old_dist, old_marker, kind)
            reports.append(report)
            markers.append(marker)
        assert len(set(markers)) == 3
    finally:
        current_tree = tree(old)
        verification = {"previous_fixture_unchanged": current_tree == original_tree,
                        "step_2_executed": False, "control_projects_retained": True,
                        "three_markers_distinct": len(set(markers)) == 3,
                        "editor_sessions_run": 0}
        for base in (copy, native):
            if base.is_dir():
                save(base / "previous-fixture-before-tree.json", original_tree)
                save(base / "previous-fixture-after-tree.json", current_tree)
                save(base / "verification.json", verification)
        assert verification["previous_fixture_unchanged"], "Existing fixture changed during preparation"
    print(json.dumps({"projects": [{key: report[key] for key in (
        "case", "project", "target", "mode", "real_directory", "content_equal_to_link_probe_except_marker")}
        for report in reports], "verification": verification}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
