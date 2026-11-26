#!/usr/bin/env python3
"""Package the existing notarized app with Tauri's installed DMG tools; never rebuild it."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys

# Import the local helper without writing into the source checkout.
sys.dont_write_bytecode = True
from release_source import manifest_source, verify_recorded_snapshot

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--identity', required=True, help='Existing Developer ID Application name or SHA-1')
parser.add_argument('--target-dir', type=Path, default=ROOT / 'target/phase7-dmg')
parser.add_argument('--evidence', type=Path, default=ROOT / 'docs/evidence/phase-7/dmg')
parser.add_argument('--manifest', type=Path, default=ROOT / 'docs/evidence/phase-7/notarization/final-artifact.json',
                    help='Verified notarized app manifest with its exact per-file SHA-256 values')
args = parser.parse_args()
target, evidence = args.target_dir.resolve(), args.evidence.resolve()
previous = json.loads(args.manifest.read_text())
built = json.loads(Path(previous['signed_build_manifest']).read_text())
source_provenance = manifest_source(previous, 'source_build_revision', verify_recorded_snapshot(built))
if previous['frozen_source'] != built['frozen_source']:
    raise SystemExit('The packaging toolchain must come from the build snapshot')
build_source = Path(built['frozen_source'])
source = Path(previous['app'])
info = plistlib.loads((source / 'Contents/Info.plist').read_bytes())
output = target / 'artifacts' / f"Kinana-{info['CFBundleShortVersionString']}-beta-arm64.dmg"
if output.exists() or (target / 'source').exists():
    parser.error('Existing packaging output is preserved; select a fresh --target-dir')

def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()

def files(app):
    if any(p.is_symlink() for p in app.rglob('*')):
        raise SystemExit('Unexpected links in the input application')
    return {str(p.relative_to(app)): sha(p) for p in sorted(app.rglob('*')) if p.is_file()}

before = files(source)
if before != previous['bundle_sha256']:
    raise SystemExit('The source app differs from the verified notarized artifact')
evidence.mkdir(parents=True, exist_ok=True)
output.parent.mkdir(parents=True, exist_ok=True)
commands = []
env = os.environ.copy()
env.pop('CI', None)  # The Finder layout is required, including on this developer machine.
for key in list(env):
    if key.startswith(('APPLE_', 'TAURI_SIGNING_')):
        del env[key]

def run(name, argv):
    print(name, flush=True)
    start = datetime.datetime.now(datetime.timezone.utc).isoformat()
    with (evidence / (name + '.txt')).open('wb') as f:
        p = subprocess.run([str(x) for x in argv], cwd=ROOT, env=env, stdout=f, stderr=subprocess.STDOUT)
    commands.append({'name': name, 'argv': [str(x) for x in argv], 'started_at': start, 'exit_code': p.returncode})
    (evidence / 'package-commands.json').write_text(json.dumps(commands, indent=2) + '\n')
    if p.returncode:
        raise SystemExit(f'{name} failed ({p.returncode}); see {evidence / (name + ".txt")}')

# Tauri's bundle command recreates .app. Use its unchanged embedded DMG script
# directly so that the input signature and already-stapled ticket are preserved.
version = json.loads((build_source / 'app/node_modules/@tauri-apps/cli/package.json').read_text())['version']
if version != '2.12.4':
    raise SystemExit('Review the embedded DMG resources before using a different Tauri version')
cli = build_source / 'app/node_modules/@tauri-apps/cli-darwin-arm64/cli.darwin-arm64.node'
binary = cli.read_bytes()
resources = [
    ('bundle_dmg', b'#!/usr/bin/env bash\n# Copyright 2019-2024 Tauri', 19299, 'dce473d510339f1a3f8009d84191ad2bf716d8cdbe12eb6e1f821771cf8f960f', 'bin/bundle_dmg.sh'),
    ('template.applescript', b'on run (volumeName)\n', 1828, '13727c7930c50be639a1eb191e893f1ebc72c270268d4611391b41410a3e862a', 'share/create-dmg/support/template.applescript'),
    ('eula-resources-template.xml', b'<?xml version="1.0" encoding="UTF-8"?>\n', 2376, '7d61d948ba7f118306d61107fead78f04e3338585e2ec729abb65d5d2e21012d', 'share/create-dmg/support/eula-resources-template.xml'),
]
tool_provenance = {'tauri_version': version, 'installed_cli': str(cli), 'installed_cli_sha256': sha(cli), 'resources': []}
for name, marker, length, expected, relative in resources:
    offset = binary.find(marker)
    while offset >= 0 and hashlib.sha256(binary[offset:offset + length]).hexdigest() != expected:
        offset = binary.find(marker, offset + 1)
    if offset < 0:
        raise SystemExit(f'The approved embedded Tauri resource changed: {name}')
    destination = target / 'tauri-tools' / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(binary[offset:offset + length])
    if name == 'bundle_dmg':
        destination.chmod(0o755)
    tool_provenance['resources'].append({'name': name, 'offset': offset, 'bytes': length, 'sha256': expected,
        'primary_source': 'https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/macos/dmg/' + name})
(evidence / 'tauri-tool-provenance.json').write_text(json.dumps(tool_provenance, indent=2) + '\n')
stage = target / 'source'
stage.mkdir()
app = stage / 'Kinana.app'
run('copy-notarized-app', ['/usr/bin/ditto', source, app])
if files(app) != before:
    raise SystemExit('The staged application differs from its verified source')
run('verify-source-signature', ['/usr/bin/codesign', '--verify', '--deep', '--strict', '--verbose=4', app])
run('verify-source-ticket', ['xcrun', 'stapler', 'validate', app])
run('create-dmg', [target / 'tauri-tools/bin/bundle_dmg.sh', '--volname', 'Kinana',
    '--volicon', app / 'Contents/Resources' / info['CFBundleIconFile'],
    '--window-size', '660', '400', '--icon-size', '128', '--text-size', '16',
    '--icon', 'Kinana.app', '180', '170', '--app-drop-link', '480', '170',
    '--hide-extension', 'Kinana.app', '--no-internet-enable', output, stage])
if files(app) != before or files(source) != before:
    raise SystemExit('Packaging changed application bytes')
run('sign-dmg', ['/usr/bin/codesign', '--force', '--timestamp', '--sign', args.identity, output])
run('verify-dmg-signature', ['/usr/bin/codesign', '--verify', '--strict', '--verbose=4', output])
run('display-dmg-signature', ['/usr/bin/codesign', '-dvvv', output])
manifest = {'source_app': str(source), 'source_build_revision': previous['source_build_revision'],
    'source_provenance': source_provenance, 'notarized_app_manifest': str(args.manifest.resolve()),
    'dmg': str(output), 'signed_dmg_sha256_before_notarization': sha(output),
    'source_tree_sha256': hashlib.sha256(json.dumps(before, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
    'bundle_file_sha256': before, 'source_app_unchanged': True, 'rebuild_performed': False,
    'window': {'width': 660, 'height': 400, 'app': [180, 170], 'applications': [480, 170]},
    'runtime_libraries_added': [], 'notarization_submitted': False, 'published': False}
(evidence / 'package.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps(manifest, indent=2), flush=True)
