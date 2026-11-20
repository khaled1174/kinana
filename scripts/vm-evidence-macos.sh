#!/bin/bash
# Collect evidence for the clean-Mac test of the Kinana beta inside a clean macOS 27
# virtual machine (docs/beta-0.2-vm-test.md). Run it inside the VM, as the test
# user, after the manual steps in that checklist.
#
#   bash vm-evidence-macos.sh <downloaded .dmg> <test project folder> [claude stream-json]
#
# The optional third argument is the stream-json output of the one authorised
# Claude Code session (step 9). With it, the script also checks the narrow
# permission rule: ~/.claude/settings.json holds the ten read-only rules, and the
# session ran status and suggest with no permission denial, with --max-turns 10 (up to 11 reported turns).
#
# Read-only: it reads the DMG, the installed app, the router files and the
# vault, and runs the bundled CLI only with read-only commands (--version,
# status, suggest, doctor, list). It writes only its own evidence folder on the
# Desktop. It never starts an agent and never uses the network.
set -u
set -o pipefail
umask 077
export PATH=/usr/bin:/bin:/usr/sbin:/sbin
DMG=${1:?usage: vm-evidence-macos.sh <dmg> <project> [transcript]}
PROJECT=${2:?usage: vm-evidence-macos.sh <dmg> <project> [transcript]}
TRANSCRIPT=${3:-}
APP=/Applications/Kinana.app
EXPECTED_CLI="$APP/Contents/MacOS/kinana"
OUT="$HOME/Desktop/kinana-vm-evidence-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir "$OUT" || exit 1
RESULTS="$OUT/checks.tsv"
printf 'check\texit\texpected\tresult\n' > "$RESULTS" || exit 1

# rec NAME EXPECTED_EXIT COMMAND... : output to NAME.txt, one row in checks.tsv.
rec() {
  local name=$1 expected=$2; shift 2
  { printf '$'; printf ' %q' "$@"; printf '\n'; } > "$OUT/$name.txt" || exit 1
  "$@" >> "$OUT/$name.txt" 2>&1
  local code=$?
  local result=FAIL
  if [ "$expected" = any ] || [ "$code" = "$expected" ]; then result=PASS; fi
  printf '%s\t%s\t%s\t%s\n' "$name" "$code" "$expected" "$result" >> "$RESULTS" || exit 1
  [ "$result" = PASS ]
}

# spctl_api NAME: Gatekeeper's "invalid API object reference" is its own
# failure (round 16), separate from the spctl exit code, with its message.
spctl_api() {
  local line
  line=$(grep -m1 -i 'invalid API object reference' "$OUT/$1.txt" | tr '\t' ' ')
  if [ -n "$line" ]; then
    printf '%s-api-object\t%s\tno invalid API object reference\tFAIL\n' "$1" "$line" >> "$RESULTS" || exit 1
  else
    printf '%s-api-object\t-\tno invalid API object reference\tPASS\n' "$1" >> "$RESULTS" || exit 1
  fi
}

rec system 0 sw_vers
uname -m >> "$OUT/system.txt"
# The clean-Mac test runs on macOS 27 (owner, round 15): a clean machine, not
# the oldest supported version. Any 27.x on Apple Silicon passes.
case "$(sw_vers -productVersion)" in 27|27.*) v=yes ;; *) v=no ;; esac
printf 'macos-27\t%s\tyes\t%s\n' "$(sw_vers -productVersion)" "$([ "$v" = yes ] && echo PASS || echo FAIL)" >> "$RESULTS" || exit 1
if [ "$(uname -m)" = arm64 ]; then a=PASS; else a=FAIL; fi
printf 'arch-arm64\t%s\tarm64\t%s\n' "$(uname -m)" "$a" >> "$RESULTS" || exit 1
rec dmg-sha256 0 shasum -a 256 "$DMG"
# Downloaded through a browser, the image must carry the quarantine flag.
rec dmg-quarantine 0 xattr -p com.apple.quarantine "$DMG"
rec dmg-stapler 0 xcrun stapler validate "$DMG"
rec dmg-spctl 0 spctl -a -t open --context context:primary-signature -vv "$DMG"
spctl_api dmg-spctl
rec app-codesign 0 /usr/bin/codesign --verify -R='anchor apple generic and identifier "io.github.khaled1174.kinana" and certificate leaf[subject.OU] = "YK6WUBTZ6S"' --deep --strict --verbose=2 "$APP" || exit 1
rec app-spctl 0 spctl --assess --type execute -vv "$APP"
spctl_api app-spctl
rec app-stapler 0 xcrun stapler validate "$APP"
rec app-version 0 /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist"
rec app-executables 0 ls -l "$APP/Contents/MacOS"
rec cli-codesign 0 /usr/bin/codesign --verify -R='anchor apple generic and identifier "kinana" and certificate leaf[subject.OU] = "YK6WUBTZ6S"' --strict --verbose=2 "$EXPECTED_CLI" || exit 1
# No manual PATH setup: the command is not on PATH, in a login shell either.
rec which-kinana 1 /bin/zsh -lc 'command -v kinana'

routers_ok=PASS
for base in .claude .agents; do
  dir="$HOME/$base/skills/kinana"
  rec "router-${base#.}-files" 0 ls -la "$dir"
  rec "router-${base#.}-skill" 0 cat "$dir/SKILL.md"
  rec "router-${base#.}-owner" 0 cat "$dir/.skillsbox-router.toml"
  if [ ! -r "$dir/SKILL.md" ] || [ ! -r "$dir/.skillsbox-router.toml" ]; then
    routers_ok=FAIL
    continue
  fi
  named=$(sed -n "s/^The Kinana CLI is \`'\(.*\)'\`\..*/\1/p" "$dir/SKILL.md")
  if [ "$named" != "$EXPECTED_CLI" ] || ! grep -Fxq "cli_path = \"$EXPECTED_CLI\"" "$dir/.skillsbox-router.toml"; then
    printf 'router-%s-bundled-cli\t-\tverified path\tFAIL\n' "$base" >> "$RESULTS" || exit 1
    exit 1
  fi
done
printf '%s\n' "$EXPECTED_CLI" > "$OUT/router-cli-path.txt" || exit 1
printf 'router-names-bundled-cli\t-\tverified path\t%s\n' "$routers_ok" >> "$RESULTS" || exit 1

rec cli-version 0 "$EXPECTED_CLI" --json --version
rec cli-doctor 0 "$EXPECTED_CLI" --json doctor
rec cli-list 0 "$EXPECTED_CLI" --json list --brief
rec cli-status 0 "$EXPECTED_CLI" --json status --project "$PROJECT"
rec cli-suggest 0 "$EXPECTED_CLI" --json suggest --project "$PROJECT"
rec project-files 0 ls -la "$PROJECT"

# Gatekeeper decisions for the first launch, from the unified log.
rec gatekeeper-log any log show --last 2h --style compact --predicate \
  'process == "syspolicyd" AND eventMessage CONTAINS[c] "Kinana"'

# check NAME EXPECTED VALUE : PASS when VALUE equals EXPECTED.
check() {
  local r=FAIL
  if [ "$3" = "$2" ]; then r=PASS; fi
  printf '%s\t%s\t%s\t%s\n' "$1" "$3" "$2" "$r" >> "$RESULTS" || exit 1
}

if [ -n "$TRANSCRIPT" ]; then
  /usr/bin/env -i PATH=/usr/bin:/bin /usr/bin/ruby --disable-gems - "$HOME/.claude/settings.json" "$TRANSCRIPT" "$OUT" "$EXPECTED_CLI" "$PROJECT" <<'SESSION'
require "json"
require "shellwords"

# Apple's system Ruby and JSON library are available independently of Xcode.
class InvalidEvidence < StandardError; end

class UniqueFields < Hash
  def []=(key, value)
    raise InvalidEvidence if key?(key)
    super
  end
end

def require_evidence(ok)
  raise InvalidEvidence unless ok
end

def parse(text)
  JSON.parse(text, object_class: UniqueFields)
end

def project_path(path, project)
  require_evidence(path.is_a?(String) && !path.empty? && !path.start_with?("~") && !path.split("/").include?(".."))
  resolved = File.realpath(path, project)
  require_evidence(resolved == project || resolved.start_with?(project + "/"))
  resolved
end

def safe_pattern(pattern)
  # Globs operate relative to the checked search root.
  require_evidence(pattern.is_a?(String) && !pattern.empty?)
  # Common extension alternatives cannot introduce new path components.
  checked = pattern.gsub(/\{[[:alnum:]_-]+(?:,[[:alnum:]_-]+)+\}/, "extension")
  require_evidence(!checked.start_with?("/", "~") && !checked.include?("..") && !checked.match?(/[{}\\]/))
end

def project_tool(name, input, project)
  keys = {"Read"=>%w[file_path offset limit pages], "Glob"=>%w[path pattern],
    "Grep"=>%w[path pattern glob type output_mode -A -B -C context -n -i head_limit offset multiline]}
  require_evidence(keys.key?(name) && (input.keys - keys[name]).empty?)
  if name == "Read"
    project_path(input.fetch("file_path"), project)
  else
    root = project_path(input.fetch("path", "."), project)
    safe_pattern(input.fetch("pattern")) if name == "Glob"
    safe_pattern(input["glob"]) if input.key?("glob")
    require_evidence(input.fetch("pattern").is_a?(String))
    # A recursive search must also stay in the project when it encounters links.
    require "find"
    Find.find(root, ignore_error: false) do |entry|
      next unless File.symlink?(entry)
      project_path(entry, project)
      require_evidence(!File.directory?(entry))
    end
  end
  name
end

def read_command(command, cli, project)
  word = /(?:[[:alnum:]_.\/=-]+|'[^'\r\n]*'|"[^"$`\\!\r\n]*")/
  require_evidence(command.is_a?(String) && command.match?(/\A[ \t]*#{word}(?:[ \t]+#{word})*[ \t]*\z/))
  path, *args = Shellwords.split(command)
  if %w[ls /bin/ls].include?(path)
    args.each do |arg|
      next if arg.match?(/\A-[laAh1dFtSr]+\z/)
      require_evidence(!arg.start_with?("-"))
      project_path(arg, project)
    end
    return "ls"
  end
  require_evidence(path == cli)
  name = args.shift
  options = {"status"=>%w[project], "suggest"=>%w[project],
    "list"=>%w[brief pinned query state category stack stage agent audit limit page],
    "show"=>[], "search"=>[]}
  require_evidence(options.key?(name))
  flags = {}
  positional = []
  until args.empty?
    arg = args.shift
    if arg.start_with?("--")
      key, value = arg[2..-1].split("=", 2)
      require_evidence((key == "json" || options[name].include?(key)) && !flags.key?(key))
      if %w[json brief pinned].include?(key)
        require_evidence(value.nil?)
        flags[key] = true
      else
        value ||= args.shift
        require_evidence(value && !value.empty? && !value.start_with?("-"))
        flags[key] = value
      end
    else
      require_evidence(!arg.empty? && !arg.start_with?("-"))
      positional << arg
    end
  end
  require_evidence(flags["json"] && positional.size == (%w[show search].include?(name) ? 1 : 0))
  require_evidence(project_path(flags.fetch("project"), project) == project) if name == "suggest" || flags.key?("project")
  {"limit"=>50, "page"=>4294967295}.each do |key, max|
    n = flags[key]
    require_evidence(flags["brief"] && n.match?(/\A[0-9]+\z/) && n.to_i.between?(1,max)) if n
  end
  require_evidence(%w[pending proposal approved original].include?(flags["state"])) if flags["state"]
  require_evidence(%w[claude codex cursor].include?(flags["agent"])) if flags["agent"]
  flags["brief"] ? "list --brief" : name
end

def valid_payload(name, payload)
  return payload.is_a?(Array) if name == "list"
  return false unless payload.is_a?(Hash) && !payload.key?("error")
  return payload["skill"].is_a?(Hash) && payload["raw_skill_md"].is_a?(String) if name == "show"
  field = {"status"=>"projects", "suggest"=>"skills", "list --brief"=>"skills", "search"=>"items"}.fetch(name)
  payload[field].is_a?(Array)
end

begin
  settings, transcript, output, cli, project = ARGV
  project = File.realpath(project)
  initialized = false
  expected = %w[status suggest list search show].flat_map do |name|
    [cli, "'#{cli}'"].map { |path| "Bash(#{path} #{name} *)" }
  end.sort
  permissions = parse(File.read(settings)).fetch("permissions")
  require_evidence(permissions.keys == ["allow"] && permissions["allow"].sort == expected)
  File.write(File.join(output, "claude-settings.json"), JSON.pretty_generate({"permissions"=>{"allow"=>expected}}) + "\n")

  session = nil
  turns = nil
  calls = {}
  completed = {}
  File.foreach(transcript) do |line|
    next if line.strip.empty?
    event = parse(line)
    sid = event["session_id"]
    require_evidence(turns.nil? && sid.is_a?(String) && !sid.empty? && (session.nil? || sid == session))
    session = sid
    kind = event["type"]
    if kind == "system" && event["subtype"] == "init"
      require_evidence(!initialized && File.realpath(event.fetch("cwd")) == project)
      initialized = true
    end
    if kind == "result"
      require_evidence(event["subtype"] == "success" && event["is_error"] == false && event["permission_denials"] == [])
      turns = event["num_turns"]
      require_evidence(turns.is_a?(Integer) && turns.between?(1, 11))
      next
    end
    next unless %w[assistant user].include?(kind)
    require_evidence(initialized && event["parent_tool_use_id"].nil?)
    blocks = event.fetch("message").fetch("content")
    next if blocks.is_a?(String)
    require_evidence(blocks.is_a?(Array))
    blocks.each do |block|
      case block.fetch("type")
      when "tool_use"
        id = block.fetch("id")
        require_evidence(kind == "assistant" && id.is_a?(String) && !id.empty? && !calls.key?(id))
        input = block.fetch("input")
        if block["name"] == "Skill"
          require_evidence(input["skill"] == "kinana" && (input.keys - %w[skill args]).empty?)
          calls[id] = "Skill"
        else
          if block["name"] != "Bash"
            calls[id] = project_tool(block["name"], input, project)
            next
          end
          require_evidence((input.keys - %w[command description]).empty?)
          calls[id] = read_command(input.fetch("command"), cli, project)
        end
      when "tool_result"
        id = block.fetch("tool_use_id")
        require_evidence(kind == "user" && calls.key?(id) && !completed.key?(id))
        require_evidence(block.fetch("is_error", false) == false)
        native = event["tool_use_result"]
        if native.is_a?(Hash)
          require_evidence(!native["interrupted"] && native.fetch("success", true) == true &&
            native.fetch("exitCode", 0) == 0)
        end
        content = block.fetch("content")
        require_evidence(content.is_a?(String) || content.is_a?(Array))
        if calls[id] == "Skill"
          require_evidence(!content.empty?)
        else
          native_read = %w[Read Glob Grep].include?(calls[id])
          require_evidence(native_read || block["is_error"] == false)
          if native_read || calls[id] == "ls"
            completed[id] = true
            next
          end
          if content.is_a?(Array)
            require_evidence(content.all? { |part| part["type"] == "text" && part["text"].is_a?(String) })
            content = content.map { |part| part["text"] }.join
          end
          payload = parse(content)
          require_evidence(valid_payload(calls[id], payload))
        end
        completed[id] = true
      end
    end
  end
  require_evidence(initialized && turns && calls.keys.sort == completed.keys.sort && (%w[Skill status suggest] - calls.values).empty?)
  summary = {"turns"=>turns, "commands"=>calls.values.reject { |c| c == "Skill" }, "success"=>true}
  File.write(File.join(output, "claude-session-summary.json"), JSON.pretty_generate(summary) + "\n")
rescue StandardError
  abort "Invalid permission rules or incomplete/unsuccessful tool evidence"
end
SESSION
  code=$?
  check session-and-permissions 0 "$code"
  if [ -e "$PROJECT/.kinana.lock" ]; then l=yes; else l=no; fi
  check project-no-lock no "$l"
fi

fails=$(awk -F'\t' '
  NR == 1 { if ($4 != "result") exit 1; next }
  NF != 4 || ($4 != "PASS" && $4 != "FAIL") { exit 1 }
  $4 == "FAIL" { failures++ }
  END { if (NR < 2) exit 1; print failures+0 }
' "$RESULTS") || exit 1
{
  echo "Evidence folder: $OUT"
  echo "Failed checks: $fails"
  awk -F'\t' 'NR>1 && $4=="FAIL" {print "  FAIL " $1 " (exit " $2 ", expected " $3 ")"}' "$RESULTS"
} | tee "$OUT/summary.txt" || exit 1
[ "$fails" = 0 ]
