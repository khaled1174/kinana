#!/bin/sh
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export PATH="$project_root/.tools/node-v22.23.5-darwin-arm64/bin:$PATH"
if [ "$(node --version)" != 'v22.23.5' ]; then
  echo 'Project-local Node 22.23.5 is required; see app/README.md.' >&2
  exit 1
fi
exec "$@"
