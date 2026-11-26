"""Offline regression checks for the standalone VM collector and release gates."""
import ast
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
COLLECTOR = (ROOT / "scripts/vm-evidence-macos.sh").read_text()
CLI = "/Applications/Kinana.app/Contents/MacOS/kinana"


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.folder = Path(scratch.name)
        self.project = self.folder / "project"
        self.project.mkdir()
        (self.project / "package.json").touch()
        self.rules = [f"Bash({prefix} {cmd} *)" for prefix in [CLI, "'" + CLI + "'"]
                      for cmd in ["status", "suggest", "list", "search", "show"]]

    def collect(self, events):
        settings = self.folder / ".claude/settings.json"
        settings.parent.mkdir(exist_ok=True)
        settings.write_text(json.dumps({"permissions":{"allow":self.rules}, "env":{"fixture":"private-value"}}, indent=2))
        stream = self.folder / "stream"
        events = [{"type":"system", "subtype":"init", "cwd":str(self.project)}] + events
        stream.write_text("\n".join(json.dumps(dict(e, session_id="one"), separators=(",",":")) for e in events))
        setup = 'HOME=$1; OUT=$1; PROJECT=$1/project; EXPECTED_CLI=$2; TRANSCRIPT=$3; RESULTS=$1/checks.tsv\n'
        setup += "printf 'check\\texit\\texpected\\tresult\\n' > \"$RESULTS\"\n"
        section = COLLECTOR[COLLECTOR.index("check() {"):]
        return subprocess.run(["/bin/bash", "-c", setup+section, "test",self.folder,CLI,stream],
                              capture_output=True,text=True).returncode

    def events(self):
        events = []
        for cmd in ["status", "suggest"]:
            events.extend([
                {"type":"assistant","message":{"content":[{"type":"tool_use","id":cmd,"name":"Bash",
                 "input":{"command":f"'{CLI}' {cmd} --project . --json"}}]}},
                {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":cmd,
                 "content":json.dumps({"projects":[],"skills":[],"note":"fixture output"}),"is_error":False}]}}
            ])
        events.extend(self.extra("Skill", {"skill":"kinana"}, "Launching skill: kinana"))
        return events + [{"type":"result","subtype":"success","is_error":False,"num_turns":4,"permission_denials":[]}]

    def extra(self, name, arguments, content):
        return [
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"extra"+name,"name":name,"input":arguments}]}},
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"extra"+name,
                "content":content, **({"is_error":False} if name == "Bash" else {})}]}}
        ]

    def test_documented_project_reads_and_turn_count(self):
        events = self.events()
        events[-1]["num_turns"] = 11
        for command, payload in [
            ("list --brief --json --stack react --stage build --limit 20 --page 2", {"skills":[]}),
            ("list --category dev --agent claude --state pending --pinned --audit safe --query test --json", []),
            ("search 'two words' --json", {"items":[]}),
            ("show one --json", {"skill":{}, "raw_skill_md":"text"}),
        ]:
            with self.subTest(command=command):
                self.assertEqual(self.collect(events[:-1]+self.extra("Bash", {"command":f"'{CLI}' {command}"}, json.dumps(payload))+events[-1:]),0)
        for name, args in [("Read",{"file_path":"package.json"}),("Glob",{"pattern":"**/*.{json,ts}"}),
                           ("Grep",{"pattern":"react","path":"."}),("Bash",{"command":"ls -la ."})]:
            self.assertEqual(self.collect(events[:-1]+self.extra(name,args,"output")+events[-1:]),0)
        events[-1]["num_turns"] = 12
        self.assertNotEqual(self.collect(events),0)

    def test_outside_project_and_unapproved_calls_fail(self):
        events = self.events()
        (self.project/"outside").symlink_to(self.folder)
        for name,args in [("Read",{"file_path":"outside"}),("Glob",{"pattern":"../*"}),
                          ("Glob",{"pattern":"*"}),("Grep",{"pattern":"test","path":".."}),
                          ("Bash",{"command":"ls -la .."}),("Write",{"file_path":"file"}),
                          ("Bash",{"command":"pwd"}),("Bash",{"command":f"'{CLI}' add file --json"}),
                          ("Bash",{"command":f"'{CLI}' use file --json"}),("Bash",{"command":"ls . && ls /"}),
                          ("Bash",{"command":f"'{CLI}' list --vault outside --json"})]:
            with self.subTest(args=args):
                self.assertNotEqual(self.collect(events[:-1]+self.extra(name,args,"[]")+events[-1:]),0)

    def test_only_correlated_successful_calls_and_rules_exported(self):
        events = self.events()
        self.assertEqual(self.collect(events), 0)
        exported = json.loads((self.folder / "claude-settings.json").read_text())
        self.assertEqual(exported, {"permissions":{"allow":sorted(self.rules)}})
        self.assertNotIn("fixture output", (self.folder / "claude-session-summary.json").read_text())
        self.assertNotEqual(self.collect(events[:1]+events[2:]), 0)
        events[1]["message"]["content"][0]["is_error"] = True
        self.assertNotEqual(self.collect(events), 0)

    def test_interrupted_or_mismatched_tool_result_is_rejected(self):
        events = self.events()
        events[1]["tool_use_result"] = {"interrupted":True}
        self.assertNotEqual(self.collect(events), 0)
        del events[1]["tool_use_result"]
        events[1]["message"]["content"][0]["tool_use_id"] = "unknown"
        self.assertNotEqual(self.collect(events), 0)

    def test_router_result_must_show_success(self):
        events = self.events()
        events[-2]["tool_use_result"] = {"success":False}
        self.assertNotEqual(self.collect(events), 0)
        del events[-2]["tool_use_result"]
        events[-2]["message"]["content"][0]["content"] = ""
        self.assertNotEqual(self.collect(events), 0)

    def test_failed_result_mentions_both_commands_with_zero_tool_calls(self):
        result = self.events()[-1]
        result.update(is_error=True, result=f"{CLI} status --project . ; {CLI} suggest --project .")
        self.assertNotEqual(self.collect([result]), 0)

    def test_missing_or_unwritable_check_log_cannot_report_success(self):
        section = COLLECTOR[COLLECTOR.index("check() {"):]
        setup = 'set -o pipefail; OUT=$1; RESULTS=$2; TRANSCRIPT=""\n'
        for path in [self.folder / "missing", self.folder]:
            run = subprocess.run(["/bin/bash", "-c", setup+section, "test",self.folder,path], capture_output=True)
            self.assertNotEqual(run.returncode, 0)
        helper = section[:section.index('if [ -n "$TRANSCRIPT" ]')]
        failure = subprocess.run(["/bin/bash", "-c", setup+helper+"check example 0 0", "test",self.folder,self.folder],capture_output=True)
        self.assertNotEqual(failure.returncode, 0)

    def test_router_and_signature_gates_block_all_cli_dispatch(self):
        # Execute the actual gate and dispatch section with regular fixture files.
        section = COLLECTOR[COLLECTOR.index("rec app-codesign"):COLLECTOR.index("# Gatekeeper decisions")]
        for failed in ["app-codesign","cli-codesign","claude","agents","missing","none"]:
            with self.subTest(failed=failed):
                for agent in ["claude","agents"]:
                    base = self.folder / ("."+agent) / "skills/kinana"
                    base.mkdir(parents=True, exist_ok=True)
                    name = CLI if failed != agent else "/fixture/wrong"
                    (base/"SKILL.md").write_text(f"The Kinana CLI is `'{name}'`.\n")
                    (base/".skillsbox-router.toml").write_text(f'cli_path = "{CLI}"\n')
                if failed == "missing":
                    shutil.rmtree(self.folder/".claude/skills/kinana")
                trace = self.folder/"dispatch"
                trace.write_text("")
                setup = '''HOME=$1; OUT=$1; RESULTS=$1/checks; EXPECTED_CLI=$2; PROJECT=.; APP=/Applications/Kinana.app
rec() {
  if [ "$1" = "$failed" ]; then return 1; fi
  case "$1" in cli-version|cli-doctor|cli-list|cli-status|cli-suggest) echo "$3" >> "$OUT/dispatch";; esac
}
failed=$3
'''
                run = subprocess.run(["/bin/bash","-c",setup+section,"test",self.folder,CLI,failed],capture_output=True)
                self.assertEqual(run.returncode == 0, failed in ["missing","none"])
                if failed == "missing":
                    self.assertIn("router-names-bundled-cli\t-\tverified path\tFAIL", (self.folder/"checks").read_text())
                    self.assertNotIn(b"sed:",run.stderr)
                self.assertEqual(trace.read_text().splitlines(), [CLI]*5 if failed in ["missing","none"] else [])


class ReleaseTests(unittest.TestCase):
    def test_invalid_minimum_exits_before_build_in_both_interpreter_modes(self):
        source = ROOT/"scripts/build-beta-macos.py"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            script = root/"scripts"/source.name
            script.parent.mkdir()
            shutil.copyfile(source,script)
            shutil.copyfile(source.with_name("release_source.py"), script.with_name("release_source.py"))
            cfg = root/"app/src-tauri"
            cfg.mkdir(parents=True)
            (cfg/"tauri.conf.json").write_text('{"version":"test"}')
            (cfg/"tauri.beta.conf.json").write_text('{"bundle":{"macOS":{"minimumSystemVersion":"1.0"}}}')
            for flags in [[], ["-O"]]:
              with self.subTest(flags=flags):
                result = subprocess.run([sys.executable,*flags,script,"--identity","fixture"],capture_output=True,text=True)
                self.assertNotEqual(result.returncode,0)
                self.assertIn("Beta configuration must specify macOS 27.0",result.stderr)
                self.assertFalse((root/"docs/evidence/phase-7/signed-build/commands.json").exists())
                self.assertFalse((script.parent/"__pycache__").exists())

        self.assertFalse(any(isinstance(node, ast.Assert) for node in ast.walk(ast.parse(source.read_text()))))


if __name__ == "__main__":
    unittest.main()
