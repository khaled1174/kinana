#!/usr/bin/env python3
"""Run WebdriverIO under a temporary identity for Kinana's application data."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import pwd
import re
import shutil
import stat
import subprocess
import tempfile
import time

BUNDLE = "io.github.khaled1174.kinana"
TEST_ID = BUNDLE + ".e2e"
TEST_NAME = "Kinana E2E"
LIBRARY_PATHS = (
    "WebKit/{}", "Application Support/{}", "Caches/{}", "HTTPStorages/{}",
    "Preferences/{}.plist", "Saved Application State/{}.savedState", "Containers/{}",
    "Logs/{}", "Cookies/{}", "Cookies/{}.binarycookies", "HTTPStorages/{}.binarycookies",
    "Caches/com.apple.WebKit.Networking/{}", "Caches/com.apple.WebKit.WebContent/{}",
)
ACCOUNT_PATHS = (
    ".kinana", ".kinana-preview", "KinanaVault", "KinanaVault-preview",
    ".kinana-write.lock", ".kinana-preview-write.lock",
    ".claude/skills", ".agents/skills", ".codex/skills", ".cursor/skills", ".pi/skills",
)
CONFIGS = {
    "isolation": "wdio.isolated.conf.mjs", "projects": "wdio.isolated.conf.mjs",
    "ui": "wdio.conf.mjs", "classification": "wdio.classification.conf.mjs",
    "scan-review": "wdio.scan-review.conf.mjs",
}
SPEC_SUITES = {
    "isolated-ui": "projects",
    "classification": "classification",
    "scan-review": "scan-review",
}


def exists(path):
    try:
        path.lstat()
        return True
    except FileNotFoundError:
        return False


def app_paths(account, bundle):
    for pattern in LIBRARY_PATHS:
        yield account / "Library" / pattern.format(bundle)
    for prefix, suffix in (("Preferences/ByHost", ".plist"),):
        folder = account / "Library" / prefix
        if exists(folder):
            yield from sorted(path for path in folder.iterdir()
                              if path.name.startswith(bundle + ".") and path.name.endswith(suffix)
                              and path.name.split(".")[-2].count("-") == 4)


def darwin_root(name):
    return Path(subprocess.check_output(
        ["/usr/bin/getconf", name], text=True).strip()).resolve()


def darwin_paths(roots, bundle):
    helper = re.compile(r"com\.apple\.WebKit\.[A-Za-z.]+\+" + re.escape(bundle))
    return {path for root in roots for path in root.iterdir()
            if path.name == bundle or helper.fullmatch(path.name)}


def snapshot(path):
    digest = hashlib.sha256()
    counts = {"files": 0, "directories": 0, "links": 0, "bytes": 0, "credential_exclusions": 0}

    def visit(entry):
        name = entry.name.casefold()
        if (name in {"auth.json", "auth.toml", "id_rsa", "id_dsa", "id_ed25519", "id_ecdsa", ".netrc", ".npmrc"}
                or name.startswith(".env") or re.search(r"(^|[._-])(auth|tokens?|credentials?|private[._-]?key|api[._-]?key|secrets?|keychain)([._-]|$)", name)
                or entry.suffix.casefold() in {".pem", ".key", ".p12", ".pfx", ".kdbx", ".jks"}):
            counts["credential_exclusions"] += 1
            return
        info = entry.lstat()
        digest.update(json.dumps([os.fspath(entry.relative_to(path)), info.st_mode,
                                  info.st_uid, info.st_gid], ensure_ascii=True).encode())
        if stat.S_ISLNK(info.st_mode):
            counts["links"] += 1
            digest.update(os.fsencode(os.readlink(entry)))
        elif stat.S_ISDIR(info.st_mode):
            counts["directories"] += 1
            for child in sorted(entry.iterdir()):
                visit(child)
        elif stat.S_ISREG(info.st_mode):
            counts["files"] += 1
            counts["bytes"] += info.st_size
            descriptor = os.open(entry, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            with os.fdopen(descriptor, "rb") as file:
                opened = os.fstat(file.fileno())
                if not stat.S_ISREG(opened.st_mode) or opened.st_ino != info.st_ino or opened.st_dev != info.st_dev:
                    raise RuntimeError(f"Entry changed during fingerprinting: {entry}")
                digest.update(hashlib.file_digest(file, "sha256").digest())
            after = entry.lstat()
            if (after.st_size, after.st_mtime_ns, after.st_ino) != (
                info.st_size, info.st_mtime_ns, info.st_ino
            ):
                raise RuntimeError(f"File changed during fingerprinting: {entry}")
        else:
            raise RuntimeError(f"Unsupported entry in fingerprint: {entry}")
        digest.update(b"\0")

    if not exists(path):
        return {"status": "absent"}
    visit(path)
    return {"status": "present", "sha256": digest.hexdigest(), **counts}


def protected_roots(account, system_roots):
    roots = {account / path for path in ACCOUNT_PATHS}
    for bundle in (BUNDLE, BUNDLE + ".preview"):
        roots.update(app_paths(account, bundle))
        roots.update(folder / bundle for folder in system_roots)
        roots.update(darwin_paths(system_roots, bundle))
    roots.update(path.resolve() for path in tuple(roots) if path.is_symlink())
    return roots


def fingerprints(roots):
    return {str(path): snapshot(path) for path in sorted(roots)}


def process_table():
    output = subprocess.check_output(["/bin/ps", "-ax", "-o", "pid=,uid=,command="], text=True)
    return {int(pid): command for line in output.splitlines()
            for pid, uid, command in [line.strip().split(maxsplit=2)]
            if int(uid) == os.getuid()}


def system_metadata(root):
    entries = {}

    def visit(entry):
        info = entry.lstat()
        entries[str(entry)] = {
            "inode": info.st_ino, "uid": info.st_uid, "size": info.st_size,
            "created": info.st_birthtime, "modified_ns": info.st_mtime_ns,
        }
        if stat.S_ISDIR(info.st_mode):
            for child in sorted(entry.iterdir()):
                visit(child)

    if exists(root):
        visit(root)
    return entries


def observe_system_writers(root, driver_pid):
    # The isolation profile denies these paths to the test process tree.
    # Capture only exact writable handles in Codex's managed installation.
    output = subprocess.check_output(["/bin/ps", "-ax", "-o", "pid=,ppid=,uid=,comm="], text=True)
    processes = {}
    for line in output.splitlines():
        fields = line.split(maxsplit=3)
        if len(fields) == 4 and int(fields[2]) == os.getuid():
            pid, parent, _, executable = fields
            processes[int(pid)] = (int(parent), Path(executable).name)
    agents = {pid: executable for pid, (_, executable) in processes.items()
              if re.fullmatch(r"codex|claude|t3 code|python[0-9.]*", executable, re.IGNORECASE)}
    if not agents:
        return {}

    def scope(pid):
        while pid in processes and pid != os.getpid() and pid != driver_pid:
            pid = processes[pid][0]
        return "test process tree" if pid in {driver_pid, os.getpid()} else "outside test process tree"

    opened = subprocess.run(
        ["/usr/sbin/lsof", "-w", "-n", "-P", "-a", "-p", ",".join(map(str, agents)), "-Fpfan"],
        capture_output=True, text=True,
    )
    result = {}
    pid = None
    access = None
    for field in opened.stdout.splitlines():
        if field.startswith("p"):
            pid = int(field[1:])
            access = None
        elif field.startswith("f"):
            access = None
        elif field.startswith("a"):
            access = field[1:]
        elif field.startswith("n") and pid in agents and access in {"w", "u"}:
            path = field[1:]
            if Path(path).is_relative_to(root):
                result.setdefault(path, []).append({
                    "pid": pid, "process": agents[pid], "scope": scope(pid),
                    "observed_at": time.time(),
                    "evidence": "open writable file descriptor",
                })
    return result


def observe(account, initial_processes, session):
    processes = process_table()
    selected = {pid: command for pid, command in processes.items()
                if str(session) in command and f"{TEST_NAME}.app/Contents/MacOS/" in command
                or pid not in initial_processes and "/com.apple.WebKit." in command}
    if not selected:
        return {}
    result = subprocess.run(["/usr/sbin/lsof", "-w", "-n", "-P", "-a", "-p",
                             ",".join(map(str, selected)), "-Fpn"], capture_output=True, text=True)
    observed = {str(pid): {"command": command, "paths": []} for pid, command in selected.items()}
    current = None
    private = str(account / "Library") + "/"
    for line in result.stdout.splitlines():
        if line.startswith("p"):
            current = observed.get(line[1:])
        elif current is not None and line.startswith("n") and (
            line[1:].startswith((private, str(session) + "/", "/private/var/folders/"))
            or BUNDLE in line
        ):
            current["paths"].append(line[1:])
    return {pid: data for pid, data in observed.items() if data["paths"]}


def sandbox_profile(account, protected, allowed):
    # Keep personal skill directories and provider state inaccessible from WDIO
    # and its direct children. macOS WebKit helpers are verified separately.
    private_agents = [account / name for name in (".claude", ".agents", ".codex", ".cursor", ".pi")]
    filters = "\n".join(f"  (subpath {json.dumps(str(path))})"
                        for path in sorted(protected | set(private_agents)))
    exceptions = "\n".join(f"    (require-not (subpath {json.dumps(str(path))}))"
                           for path in sorted(allowed))
    return ("(version 1)\n(allow default)\n"
            f"(deny file-read* file-write*\n{filters})\n"
            f"(deny file-write* (require-all (subpath {json.dumps(str(account))})\n"
            f"{exceptions}))\n")


def verify_app(app):
    with (app / "Contents" / "Info.plist").open("rb") as file:
        info = plistlib.load(file)
    if app.name != TEST_NAME + ".app" or info["CFBundleIdentifier"] != TEST_ID:
        raise RuntimeError("Expected a Kinana E2E bundle with the .e2e identifier")
    code = subprocess.run(["/usr/bin/codesign", "-d", "--verbose=4", str(app)],
                          capture_output=True, text=True, check=True).stderr
    lines = code.splitlines()
    if ("Signature=adhoc" not in lines or "TeamIdentifier=not set" not in lines
            or "Identifier=" + TEST_ID not in lines
            or any(line.startswith(("Authority=", "Timestamp=")) for line in lines)):
        raise RuntimeError("Only the .e2e ad-hoc test signature is permitted")
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    return info["CFBundleExecutable"], lines


def system_cleanup(paths):
    results = []
    for path in sorted(paths):
        info = path.lstat()
        if info.st_uid != os.getuid() or not stat.S_ISDIR(info.st_mode):
            raise RuntimeError(f"Unexpected owner or entry type for .e2e directory: {path}")
        usage = subprocess.run(["/usr/bin/du", "-sk", str(path)], capture_output=True, text=True)
        result = {
            "path": str(path), "uid": info.st_uid, "allocated_kib": (
                int(usage.stdout.split()[0]) if usage.returncode == 0 else None
            ), "directory_stat_bytes": info.st_size, "size_error": usage.stderr.strip() or None,
        }
        try:
            shutil.rmtree(path)
            result["status"] = "removed"
        except OSError as error:
            result.update(status="kept", error=str(error), errno=error.errno)
        results.append(result)
    return results


def run(args):
    account = Path(pwd.getpwuid(os.getuid()).pw_dir).resolve()
    system_roots = tuple(darwin_root("DARWIN_USER_" + suffix) for suffix in ("CACHE_DIR", "TEMP_DIR"))
    if os.getuid() == 0:
        raise RuntimeError("Run the isolation verifier as the current non-root account")
    allowed = set(app_paths(account, TEST_ID))
    for path in sorted(allowed):
        if exists(path):
            raise RuntimeError(f"Pre-existing .e2e data would make verification ambiguous: {path}")

    name, signature = verify_app(args.app)
    protected = protected_roots(account, system_roots)
    before = fingerprints(protected)
    process_baseline = process_table()
    codex_skills = account / ".codex/skills"
    codex_system = codex_skills / ".system"
    system_before = system_metadata(codex_system)
    system_writers = {}
    # Fixed-location lookup intentionally rejects anything under world-writable
    # /private/tmp. The Finder-path fixture needs the user's private temp root.
    session_parent = system_roots[1] if args.spec == "agents-outside-path" else Path("/private/tmp")
    run_root = Path(tempfile.mkdtemp(prefix="kinana-ui-", dir=session_parent))
    report = {"suite": args.suite, "uid": os.getuid(), "session": str(run_root),
              "signature": signature, "before": before, "after": None,
              "application_paths": [], "processes": {}, "archived": [], "accepted": False,
              "darwin_e2e_before": sorted(map(str, darwin_paths(system_roots, TEST_ID)))}
    if args.spec:
        report["spec"] = args.spec
    evidence = args.evidence.resolve()
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    log = evidence / "webdriverio.log"
    try:
        for name_dir in ("home", "tmp", "evidence", "bin"):
            (run_root / name_dir).mkdir()
        test_bundle = run_root / args.app.name
        shutil.copytree(args.app, test_bundle, symlinks=True)
        binary = test_bundle / "Contents" / "MacOS" / name
        test_cli = run_root / "bin" / "kinana"
        shutil.copy2(args.cli, test_cli)
        # The native router preview requires a real sidecar beside the app.
        # Provision only the temporary test bundle and retain its ad-hoc identity.
        shutil.copy2(test_cli, binary.parent / "kinana")
        subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none",
                        "--preserve-metadata=identifier,flags", str(test_bundle)], check=True,
                       capture_output=True, text=True)
        _, report["signature"] = verify_app(test_bundle)
        profile = run_root / "test.sb"
        profile.write_text(sandbox_profile(account, protected, allowed))
        env = {
            "HOME": str(run_root / "home"), "TMPDIR": str(run_root / "tmp") + "/",
            "PATH": str(args.node.parent) + ":/usr/bin:/bin:/usr/sbin:/sbin",
            "CI": "true", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_AUTHOR_NAME": "Kinana test", "GIT_COMMITTER_NAME": "Kinana test",
            "GIT_AUTHOR_EMAIL": "kinana-test@example.invalid",
            "GIT_COMMITTER_EMAIL": "kinana-test@example.invalid",
            "KINANA_UI_BASE": str(run_root), "KINANA_UI_HOME": str(run_root / "home"),
            "KINANA_UI_TMP": str(run_root / "tmp"), "KINANA_UI_EVIDENCE": str(run_root / "evidence"),
            "KINANA_E2E_BINARY": str(binary), "KINANA_TEST_CLI": str(test_cli),
            "KINANA_ISOLATED_SUITE": args.suite,
        }
        repo_app = Path(__file__).resolve().parents[1] / "app"
        if args.spec:
            env["KINANA_ISOLATED_SPEC"] = args.spec
        if args.spec in {"onboarding", "fresh-default"}:
            env["KINANA_UI_EMPTY"] = "1"
        if args.spec == "migration":
            env["KINANA_UI_LEGACY"] = "1"
        if args.spec in {"agents-unavailable", "agents-outside-path"}:
            env["KINANA_UI_AGENT_HOME_CASE"] = (
                "missing" if args.spec == "agents-unavailable" else "outside-path"
            )
        config = CONFIGS[args.suite]
        command = ["/usr/bin/sandbox-exec", "-f", str(profile), str(args.node),
                   "node_modules/@wdio/cli/bin/wdio.js", "run", config]
        if args.spec:
            command.extend(["--spec", f"./tests/{args.spec}.e2e.mjs"])
        with log.open("w") as output:
            driver = subprocess.Popen(command, cwd=repo_app, env=env,
                                      stdout=output, stderr=subprocess.STDOUT)
            next_system_sample = time.monotonic()

            def sample_system_writers():
                nonlocal next_system_sample
                if time.monotonic() < next_system_sample:
                    return
                for path, writers in observe_system_writers(codex_system, driver.pid).items():
                    saved = system_writers.setdefault(path, {})
                    for writer in writers:
                        saved[writer["pid"]] = writer
                next_system_sample = time.monotonic() + 1

            while driver.poll() is None:
                sample_system_writers()
                for pid, observation in observe(account, process_baseline, run_root).items():
                    saved = report["processes"].setdefault(
                        pid, {"command": observation["command"], "paths": []})
                    saved["paths"] = sorted(set(saved["paths"]) | set(observation["paths"]))
                time.sleep(0.25)
        report["webdriverio_exit"] = driver.returncode
        if any(str(binary) in command for command in process_table().values()):
            raise RuntimeError("Test binary is still running after WebdriverIO exited")
        settle_until = time.monotonic() + 2
        while time.monotonic() < settle_until:
            sample_system_writers()
            time.sleep(0.25)
        report["after"] = fingerprints(protected | protected_roots(account, system_roots))
        report["changed"] = sorted(path for path in report["before"].keys() | report["after"].keys()
                                   if report["before"].get(path) != report["after"].get(path))
        report["codex_system_changes"] = []
        if str(codex_skills) in report["changed"]:
            system_after = system_metadata(codex_system)
            report["codex_system_changes"] = [
                {"path": path, "before": system_before.get(path), "after": system_after.get(path),
                 "observed_writers": list(system_writers.get(path, {}).values()),
                 "attribution": "open writable handle observed" if path in system_writers else "unattributed"}
                for path in sorted(system_before.keys() | system_after.keys())
                if system_before.get(path) != system_after.get(path)
            ]
        report["observed_protected_paths"] = sorted({
            item for pid in report["processes"].values() for item in pid["paths"]
            if any(Path(item).is_relative_to(path) for path in protected)
        })

        archived = run_root / "macos-data"
        for path in sorted(set(app_paths(account, TEST_ID))):
            if exists(path):
                target = archived / path.relative_to(account / "Library")
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(str(path), str(target))
                report["archived"].append(str(path.relative_to(account / "Library")))
        report["darwin_e2e_cleanup"] = system_cleanup(darwin_paths(system_roots, TEST_ID))
        report["darwin_e2e_remaining"] = sorted(map(str, darwin_paths(system_roots, TEST_ID)))
        if any(exists(path) for path in app_paths(account, TEST_ID)):
            raise RuntimeError("Test Library paths were recreated after archiving")
        report["library_e2e_remaining"] = []
        report["application_paths"] = sorted(
            str(path.relative_to(run_root)) for path in (run_root / "home").rglob("desktop.toml"))
        if report["changed"]:
            raise RuntimeError("Protected data changed; see changed paths in isolation.json")
        if report["observed_protected_paths"]:
            raise RuntimeError("Test or new WebKit process accessed a protected path")
        if driver.returncode:
            raise RuntimeError(f"WebdriverIO exited with status {driver.returncode}")
        if args.suite in {"isolation", "projects"} and not report["application_paths"]:
            raise RuntimeError("The first-run test did not create a temporary desktop.toml")
        if not any(item.startswith("WebKit/") for item in report["archived"]):
            raise RuntimeError("Could not prove WebKit data locations")
        report["accepted"] = True
    finally:
        report["suite_output"] = [p.name for p in (run_root / "evidence").glob("*")]
        for path in (run_root / "evidence").glob("*"):
            if path.is_file() and path.suffix in (".json", ".png", ".log", ".txt", ".html"):
                shutil.copy2(path, evidence / path.name)
        (evidence / "isolation.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"Evidence: {evidence}\nTemporary session: {run_root}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--cli", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--node", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--suite", choices=CONFIGS, default="isolation")
    app_tests = Path(__file__).resolve().parents[1] / "app" / "tests"
    specs = sorted(p.name.removesuffix(".e2e.mjs") for p in app_tests.glob("*.e2e.mjs")
                   if p.name != "classification-live.e2e.mjs")
    parser.add_argument("--spec", choices=specs)
    args = parser.parse_args()
    if args.spec:
        args.suite = SPEC_SUITES.get(args.spec, "ui")
    run(args)


if __name__ == "__main__":
    main()
