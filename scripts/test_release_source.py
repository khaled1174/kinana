"""Offline tests for the source snapshot; never sign or launch a binary."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

# Import the local helper without writing into the source checkout.
sys.dont_write_bytecode = True
from release_source import manifest_source, reject_external_inputs, snapshot, verify_snapshot


class ReleaseSourceTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.base = Path(temp.name).resolve()
        self.repo = self.base / "repo"
        self.repo.mkdir()
        self.git("init", "-q")
        for name, content in {
            ".gitignore": "app/.env*\n",
            ".node-version": "22.23.5\n",
            "Cargo.lock": "",
            "Cargo.toml": "[workspace]\n",
            "LICENSE": "test\n",
            "THIRD_PARTY_NOTICES.md": "test\n",
            "app/package-lock.json": '{"packages":{}}',
            "crates/core/Cargo.toml": "[package]\nname='fixture'\n",
            "router-skill/SKILL.md": "test\n",
            "scripts/run.py": "test\n",
        }.items():
            destination = self.repo / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(content)
        self.commit()
        self.frozen = self.base / "snapshot"

    def git(self, *argv):
        return subprocess.check_output(
            ["/usr/bin/git", "-C", self.repo, "-c", "user.name=Fixture",
             "-c", "user.email=fixture@example.invalid", "-c", "core.autocrlf=false",
             "-c", "core.hooksPath=/dev/null", *argv],
            env={"PATH": "/usr/bin:/bin", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null"},
        )

    def commit(self):
        self.git("add", ".")
        self.git("commit", "-qm", "synthetic input")

    def test_committed_source_and_manifest_remain_stable(self):
        provenance, files = snapshot(self.repo, self.frozen)
        self.assertEqual(provenance["revision"], self.git("rev-parse", "HEAD").decode().strip())
        self.assertEqual(provenance["locks"]["Cargo.lock"], hashlib.sha256(b"").hexdigest())
        self.assertEqual(manifest_source({"revision": provenance["revision"], "source_provenance": provenance},
                                         "revision"), provenance)
        (self.repo / "Cargo.lock").write_text("changed on the original checkout")
        (self.frozen / "app/dist").mkdir()
        (self.frozen / "app/dist/index.html").write_text("generated")
        verify_snapshot(self.frozen, files, generated=True)
        (self.frozen / "Cargo.lock").write_text("changed in the build snapshot")
        with self.assertRaisesRegex(ValueError, "snapshot changed: Cargo.lock"):
            verify_snapshot(self.frozen, files, generated=True)

    def test_dirty_and_untracked_checkouts_are_rejected(self):
        path = self.repo / "untracked"
        path.touch()
        with self.assertRaisesRegex(ValueError, "clean Git checkout"):
            snapshot(self.repo, self.frozen)
        path.unlink()
        (self.repo / "Cargo.lock").write_text("dirty")
        with self.assertRaisesRegex(ValueError, "clean Git checkout"):
            snapshot(self.repo, self.frozen)
        self.assertFalse(self.frozen.exists())

    def test_hidden_worktree_changes_and_sparse_paths_are_rejected(self):
        for flag in ("--assume-unchanged", "--skip-worktree"):
            with self.subTest(flag=flag):
                self.git("update-index", flag, "Cargo.lock")
                with self.assertRaisesRegex(ValueError, "complete Git index"):
                    snapshot(self.repo, self.frozen)
                self.git("update-index", "--no-" + flag[2:], "Cargo.lock")

    def test_source_symlinks_and_untracked_external_configuration_are_rejected(self):
        outside = self.base / "external"
        outside.write_text("fixture")
        (self.repo / "app/.env.local").symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "Local build configuration"):
            reject_external_inputs(self.repo, self.base / "cargo")
        (self.repo / "app/.env.local").unlink()
        (self.repo / "app/external").symlink_to(outside)
        self.commit()
        with self.assertRaisesRegex(ValueError, "Non-regular release input"):
            snapshot(self.repo, self.frozen)

    def test_dependency_outside_the_snapshot_is_rejected(self):
        (self.repo / "Cargo.toml").write_text("[dependencies]\nexternal={ path='../outside' }\n")
        self.commit()
        with self.assertRaisesRegex(ValueError, "Cargo path dependency"):
            snapshot(self.repo, self.frozen)

    def test_provenance_mismatch_is_rejected_between_stages(self):
        provenance, _ = snapshot(self.repo, self.frozen)
        other = json.loads(json.dumps(provenance))
        other["locks"]["Cargo.lock"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "mismatched release source provenance"):
            manifest_source({"revision": provenance["revision"], "source_provenance": other},
                            "revision", provenance)


if __name__ == "__main__":
    unittest.main()
