#!/bin/sh
# Runs a command (normally `cargo test`) where no real agent CLI can be found.
#
# Since 0.3 `classify --agent auto` (the CLI default) starts Claude, so a test
# that relies on the default could spend the owner's subscription if `claude`
# were reachable. Kinana looks for agents on PATH, in ~/.local/bin and in the
# fixed install directories; this script gives the command a private HOME and
# a PATH with only the toolchain and the system directories, then refuses to
# start if `claude`, `codex` or `pi` is still reachable in any of them.
set -eu
real_home=$HOME
export CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}"
sandbox=$(mktemp -d "${TMPDIR:-/tmp}/kinana-no-agents.XXXXXX")
trap 'rm -rf "$sandbox"' EXIT INT TERM
# The real path: tests refuse a HOME reached through a symbolic link (/var).
sandbox=$(cd "$sandbox" && pwd -P)
export HOME="$sandbox"
export PATH="$CARGO_HOME/bin:/usr/bin:/bin:/usr/sbin:/sbin"
# The tests' own git commands get the settings Kinana passes to its own
# (crates/core/src/gitlog.rs: gc.auto=0, maintenance.auto=false): no
# background housekeeping repacks a vault while a test compares snapshots
# (owner, 2026-11-19). Kinana clears these variables for the git it starts
# from sources (crates/core/tests/sources.rs checks that).
export GIT_CONFIG_COUNT=2
export GIT_CONFIG_KEY_0=gc.auto GIT_CONFIG_VALUE_0=0
export GIT_CONFIG_KEY_1=maintenance.auto GIT_CONFIG_VALUE_1=false
directories="$HOME/.local/bin /opt/homebrew/bin /usr/local/bin $(echo "$PATH" | tr ':' ' ')"
if [ -r /etc/paths ]; then directories="$directories $(cat /etc/paths)"; fi
for file in /etc/paths.d/*; do
  [ -r "$file" ] && directories="$directories $(cat "$file")"
done
for directory in $directories; do
  for agent in claude codex pi; do
    if [ -e "$directory/$agent" ]; then
      echo "Refusing to run: a real '$agent' is reachable at $directory/$agent" >&2
      exit 1
    fi
  done
done
"$@"
