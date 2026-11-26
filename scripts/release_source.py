"""Private, commit-bound release input snapshots and manifest checks."""
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tarfile
import tomllib

ROOT_FILES = {
    b".node-version", b"Cargo.lock", b"Cargo.toml", b"LICENSE", b"THIRD_PARTY_NOTICES.md",
}
ROOT_DIRS = (b"app/", b"crates/", b"router-skill/", b"scripts/")
GENERATED_DIRS = {"app/node_modules", "app/dist", "app/src-tauri/gen"}
SIDECAR = "app/src-tauri/binaries/kinana-aarch64-apple-darwin"
LOCKS = {"Cargo.lock", "app/package-lock.json"}


def sha(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def git(root, *args):
    return subprocess.check_output(
        ["/usr/bin/git", "-C", root, *args], env={
            "PATH": "/usr/bin:/bin", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_OPTIONAL_LOCKS": "0", "GIT_NO_REPLACE_OBJECTS": "1",
        },
    )


def reject_external_inputs(root, cargo_home):
    cargo_locations = {cargo_home, *(parent / ".cargo" for parent in (root, *root.parents))}
    for directory in sorted(cargo_locations):
        if any(os.path.lexists(directory / name) for name in ("config", "config.toml")):
            raise ValueError(f"External Cargo configuration is not a release input: {directory}")
    for directory in (root, root / "app"):
        if not directory.is_dir():
            continue
        for entry in directory.iterdir():
            if entry.name in {".env", ".npmrc"} or entry.name.startswith(".env."):
                raise ValueError(f"Local build configuration is not a release input: {entry}")


def paths_from_tree(root, revision):
    entries = {}
    for entry in git(root, "ls-tree", "-rz", "--full-tree", revision).split(b"\0"):
        if not entry:
            continue
        meta, path = entry.split(b"\t", 1)
        if path not in ROOT_FILES and not path.startswith(ROOT_DIRS):
            continue
        mode, kind, oid = meta.decode("ascii").split()
        if kind != "blob" or mode not in {"100644", "100755"}:
            raise ValueError(f"Non-regular release input: {os.fsdecode(path)}")
        # Test, generated and tool files cannot be consumed by a release build
        # unless they are regular committed inputs outside generated roots.
        name = os.fsdecode(path)
        if name == SIDECAR or any(name == p or name.startswith(p + "/") for p in GENERATED_DIRS):
            raise ValueError(f"Generated input is tracked: {name}")
        entries[name] = (mode, oid)
    if not LOCKS <= entries.keys():
        raise ValueError("The commit must track both release lockfiles")
    return entries


def clean_revision(root):
    flags = git(root, "ls-files", "-v", "-z").split(b"\0")
    if any(entry and (entry[:1].islower() or entry.startswith(b"S ")) for entry in flags):
        raise ValueError("Release requires a complete Git index, without hidden working-tree changes")
    if git(root, "status", "--porcelain=v1", "--untracked-files=all"):
        raise ValueError("Release requires a clean Git checkout, including untracked files")
    return git(root, "rev-parse", "--verify", "HEAD^{commit}").decode().strip()


def snapshot(root, destination):
    revision = clean_revision(root)
    entries = paths_from_tree(root, revision)
    object_format = git(root, "rev-parse", "--show-object-format").decode().strip()
    archive = git(root, "archive", "--format=tar", revision, "--",
                  *sorted(os.fsdecode(path) for path in ROOT_FILES), "app", "crates", "router-skill", "scripts")
    destination.mkdir(mode=0o700)
    observed = {}
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
        for member in tar:
            relative = PurePosixPath(member.name)
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("Unsafe release snapshot member")
            if member.isdir():
                continue
            name = relative.as_posix()
            if name not in entries or name in observed or not member.isfile():
                raise ValueError(f"Unexpected release archive member: {name}")
            mode, oid = entries[name]
            with tar.extractfile(member) as handle:
                content = handle.read()
            blob = hashlib.new(object_format, f"blob {len(content)}\0".encode() + content)
            if blob.hexdigest() != oid:
                raise ValueError(f"Archive bytes differ from the commit: {name}")
            path = destination / name
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as handle:
                handle.write(content)
            path.chmod(int(mode[-3:], 8))
            observed[name] = (mode, hashlib.sha256(content).hexdigest())
    if entries.keys() != observed.keys():
        raise ValueError("The release archive omitted a committed input")
    if clean_revision(root) != revision:
        raise ValueError("The checkout revision changed during snapshot creation")
    provenance = {
        "revision": revision,
        "tree_sha256": tree_digest(observed),
        "locks": {name: observed[name][1] for name in sorted(LOCKS)},
    }
    verify_snapshot(destination, observed)
    validate_local_dependencies(destination)
    return provenance, observed



def validate_local_dependencies(source):
    """Reject dependency resolution that could read outside the source snapshot."""
    def check(value, base):
        if not isinstance(value, dict):
            return
        for name, nested in value.items():
            if name == "path" and isinstance(nested, str):
                resolved = (base / nested).resolve()
                if not resolved.is_relative_to(source) or not resolved.exists():
                    raise ValueError(f"Cargo path dependency is outside the snapshot or missing: {nested}")
            elif isinstance(nested, dict):
                check(nested, base)

    for path in source.rglob("Cargo.toml"):
        with path.open("rb") as handle:
            config = tomllib.load(handle)
        for table in ("dependencies", "dev-dependencies", "build-dependencies", "target", "patch", "replace", "workspace"):
            check(config.get(table), path.parent)
    lock = json.loads((source / "app/package-lock.json").read_text())
    for name, package in lock.get("packages", {}).items():
        resolved = package.get("resolved", "")
        if package.get("link") or resolved.startswith(("file:", "link:", "../", "/")):
            raise ValueError(f"Local npm dependency is not a supported release input: {name}")

def verify_snapshot(root, manifest, generated=False):
    observed = {}
    for directory, dirs, files in os.walk(root):
        for name in (*dirs, *files):
            path = Path(directory) / name
            relative = path.relative_to(root).as_posix()
            mode = path.lstat().st_mode
            if generated and relative in GENERATED_DIRS and stat.S_ISDIR(mode):
                dirs.remove(name)
            elif generated and relative == SIDECAR and stat.S_ISREG(mode):
                continue
            elif stat.S_ISREG(mode):
                observed[relative] = ("100755" if mode & stat.S_IXUSR else "100644", sha(path))
            elif not stat.S_ISDIR(mode):
                raise ValueError(f"Non-regular source input: {relative}")
    if observed != manifest:
        changed = sorted(path for path in observed.keys() | manifest.keys()
                         if observed.get(path) != manifest.get(path))
        raise ValueError(f"Release source snapshot changed: {', '.join(changed[:8])}")



def tree_digest(files):
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def verify_recorded_snapshot(build):
    source = manifest_source(build, "revision")
    files = json.loads(Path(build["source_file_manifest"]).read_text())
    if tree_digest(files) != source["tree_sha256"]:
        raise ValueError("Build file manifest differs from the recorded source tree")
    if {name: files[name][1] for name in LOCKS} != source["locks"]:
        raise ValueError("Build file manifest differs from the recorded lockfiles")
    root = Path(build["frozen_source"])
    verify_snapshot(root, {name: tuple(data) for name, data in files.items()}, generated=True)
    return source

def manifest_source(manifest, revision_field, upstream=None):
    source = manifest["source_provenance"]
    if (
        not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", source["revision"])
        or source["revision"] != manifest[revision_field]
        or set(source["locks"]) != LOCKS
        or not all(re.fullmatch(r"[0-9a-f]{64}", hash_)
                   for hash_ in (source["tree_sha256"], *source["locks"].values()))
        or (upstream is not None and source != upstream)
    ):
        raise ValueError("Invalid or mismatched release source provenance")
    return source
