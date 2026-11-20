#!/usr/bin/env python3
"""Prepare a retained, inert Cursor editor probe; never remove it automatically.

Cursor's real `use` target supplies the documented shared .agents/skills directory. No Cursor CLI
call, provider login, global skill installation, or original-content edit.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shlex
import stat
import subprocess

ROOT = Path(__file__).resolve().parents[2]
NAME = "kinana-cursor-editor-probe"
QUESTION = (
    "From the skill descriptions already available to you, return only the "
    f"EDITOR_PROBE value for {NAME}. If its description is not available, "
    "return NOT_LOADED. Do not read files, invoke skills, or use any tools."
)


def tree(root):
    result = {}
    for at, directories, files in os.walk(root, followlinks=False):
        for name in sorted(directories + files):
            path = Path(at) / name
            entry = {"mode": stat.S_IMODE(path.lstat().st_mode)}
            if path.is_symlink():
                entry.update(kind="symlink", target=os.readlink(path))
            elif path.is_dir():
                entry["kind"] = "directory"
            else:
                entry.update(kind="file", sha256=hashlib.sha256(path.read_bytes()).hexdigest())
            result[str(path.relative_to(root))] = entry
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--agent", choices=["cursor", "codex"], default="cursor")
    args = parser.parse_args()
    base = args.directory.expanduser().absolute()
    # Refuse reuse rather than clearing or adopting a pre-existing directory.
    base.mkdir(mode=0o700)
    base = base.resolve(strict=True)
    home, vault, project, source = (base / name for name in ("home", "vault", "project", "source"))
    for path in (home, project, source):
        path.mkdir()
    binary = ROOT / "target/debug/kinana"
    env = {**os.environ, "HOME": str(home), "USERPROFILE": str(home)}
    commands = []

    def run(argv):
        completed = subprocess.run(
            [str(value) for value in argv], env=env, text=True,
            capture_output=True, timeout=60,
        )
        commands.append({"argv": [str(value) for value in argv], "exit": completed.returncode,
                         "stdout": completed.stdout, "stderr": completed.stderr})
        (base / "commands.json").write_text(json.dumps(commands, ensure_ascii=False, indent=2) + "\n")
        if completed.returncode:
            raise RuntimeError(commands[-1])
        return completed.stdout

    def box(*arguments):
        return json.loads(run([binary, "--vault", vault, "--json", *arguments]))

    run(["/usr/bin/git", "-c", "core.hooksPath=/dev/null", "init", "-q", "--template=", project])
    (project / "README.md").write_text("Owned manual Cursor editor test. No executable skill content.\n")
    baseline = tree(project)
    (base / "before-use-tree.json").write_text(json.dumps(baseline, indent=2) + "\n")
    box("init")
    original = (
        f"---\nname: {NAME}\ndescription: Review spacing in native interfaces.\n---\n"
        "# Spacing reference\n\nReview rhythm and spacing in interface layouts.\n"
    )
    (source / "SKILL.md").write_text(original)
    box("add", source)
    marker = "CURSOR_EDITOR_" + secrets.token_hex(16).upper()
    improved = f"Review native interface spacing when checking layout rhythm. EDITOR_PROBE={marker}"
    proposal = {"category": "other", "tags": ["probe"], "role": "Review native spacing",
                "when_to_use": "A manual editor loading check", "when_not": "Other tasks",
                "improved_description": improved, "overlaps_with": [], "confidence": 1.0}
    metadata = vault / "meta" / (NAME + ".toml")
    metadata.write_text(metadata.read_text() + "\n[proposal]\n" + "".join(
        f"{key} = {json.dumps(value, ensure_ascii=False)}\n" for key, value in proposal.items()))
    for arguments in (
        ["add", "--", "meta/" + NAME + ".toml"],
        ["-c", "user.name=Kinana Test", "-c", "user.email=test@localhost",
         "commit", "-q", "-m", "Prepare controlled editor loading proposal"],
    ):
        run(["/usr/bin/git", "-C", vault, "-c", "core.hooksPath=/dev/null", *arguments])
    box("review", NAME, "--accept")
    assert (vault / "skills" / NAME / "SKILL.md").read_text() == original
    dist = (vault / "dist" / NAME / "SKILL.md").read_text()
    assert marker in dist.split("---\n")[1]
    assert marker not in dist.split("---\n", 2)[2] and marker not in QUESTION
    used = box("use", NAME, "--project", project, "--agent", args.agent, "--mode", "link")
    target = project / ".agents/skills" / NAME
    assert target.is_symlink() and target.resolve() == vault / "dist" / NAME
    assert not (project / ".claude").exists() and not (project / ".cursor").exists()
    status = box("status", "--project", project)
    assert status["projects"][0]["items"][0]["state"] == "current"
    second_argv = ["env", f"HOME={home}", f"USERPROFILE={home}", str(binary),
                   "--vault", str(vault), "--json", "use", NAME, "--project", str(project),
                   "--agent", "claude", "--mode", "link"]
    second = base / "step-2-add-claude.sh"
    second.write_text("#!/bin/sh\nset -eu\nexec " + shlex.join(second_argv) + "\n")
    second.chmod(0o700)
    (base / "question.txt").write_text(QUESTION + "\n")
    (base / "expected.json").write_text(json.dumps({"marker": marker, "description_original":
        "Review spacing in native interfaces.", "description_approved": improved}, indent=2) + "\n")
    (base / "after-use-tree.json").write_text(json.dumps(tree(project), indent=2) + "\n")
    report = {"project": str(project), "vault": str(vault), "skill": NAME,
              "activation_agent": args.agent, "target": str(target), "mode": "link",
              "marker_only_in_approved_description": True, "original_unchanged": True,
              "step_2_executed": False, "step_2": str(second), "question": QUESTION,
              "retained_until_owner_results": True, "use": used, "status": status}
    (base / "prepared.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in ("use", "status")},
                     ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
