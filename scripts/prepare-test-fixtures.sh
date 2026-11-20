#!/bin/sh
# Materialize synthetic inputs at the legacy test paths for the 0.2 source.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
fixture="$root/tests/fixtures/public-release"

seed() {
  source="$fixture/$1"
  dest="$root/docs/evidence/$2"
  if [ -e "$dest" ] || [ -L "$dest" ]; then
    cmp -s "$source" "$dest" || {
      printf 'Existing test input differs: %s\n' "$dest" >&2
      exit 1
    }
    return
  fi
  mkdir -p "$(dirname -- "$dest")"
  cp "$source" "$dest"
}

seed claude-help.txt phase-3/claude-help.txt
seed initial-proposal.json phase-6/batch-2b/live-proposal.json
seed revised-proposal.json phase-6/pre-3-core/live-trigger-proposal.json
seed presentation-proposal.json phase-6/batch-2/live-proposal.json
