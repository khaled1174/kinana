#!/usr/bin/env python3
"""Build and Developer-ID-sign local Apple Silicon beta artifacts. Never notarize.

The CLI ships inside the app (Kinana.app/Contents/MacOS/kinana, Tauri
bundle.externalBin; owner decision 2026-11-17). It is built from the same
commit first, signed before the app (inside out), and there is no separate
CLI archive any more."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import plistlib
import pwd
import re
import shlex
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--identity', required=True, help='Existing Developer ID Application name or SHA-1')
parser.add_argument('--evidence', type=Path, default=ROOT / 'docs/evidence/phase-7/signed-build')
parser.add_argument('--target-dir', type=Path, default=ROOT / 'target/phase7-beta', help='Fresh candidate build directory')
args = parser.parse_args()
target = args.target_dir.resolve()
base_config = json.loads((ROOT / 'app/src-tauri/tauri.conf.json').read_text())
candidate_version = base_config['version']
for name in [f'Kinana-{candidate_version}-beta-arm64.zip']:
    if (target / 'artifacts' / name).exists():
        parser.error('An existing candidate is preserved; use --target-dir with a fresh directory')
evidence = args.evidence.resolve()
evidence.mkdir(parents=True, exist_ok=True)
env = os.environ.copy()
# Tauri must never discover ambient notary credentials during this build.
for key in list(env):
    if key.startswith(('APPLE_', 'TAURI_SIGNING_', 'KINANA_E2E_', 'SKILLSBOX_E2E_')):
        del env[key]
# The oldest macOS the beta is actually tested on (owner, round 15): the app,
# the CLI and Info.plist all name it, and no older system is claimed.
MINIMUM_MACOS = '27.0'
if not (json.loads((ROOT / 'app/src-tauri/tauri.beta.conf.json').read_text())['bundle']['macOS']['minimumSystemVersion'] == MINIMUM_MACOS):
    raise SystemExit('Beta configuration must specify macOS 27.0')
env.update(CARGO_TARGET_DIR=str(target), MACOSX_DEPLOYMENT_TARGET=MINIMUM_MACOS)
account = pwd.getpwuid(os.getuid())
home = Path(account.pw_dir).resolve()
cargo_home = Path(env.get("CARGO_HOME", str(Path.home() / ".cargo"))).resolve()
prefixes = {home: "/home/build", Path.home().resolve(): "/home/build",
            cargo_home: "/cargo", ROOT: "/src/kinana", target: "/build/kinana"}
# rustc uses the last matching prefix. Apply more specific mappings last.
prefixes = sorted(prefixes.items(), key=lambda item: len(item[0].parts))
rustflags = env.get("CARGO_ENCODED_RUSTFLAGS", "").split("\x1f")
if not rustflags[0]:
    rustflags = shlex.split(env.get("RUSTFLAGS", ""))
rustflags.extend(f"--remap-path-prefix={path}={replacement}" for path, replacement in prefixes)
env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(rustflags)
cflags = shlex.split(env.get("CFLAGS", ""))
cflags.extend(f"-ffile-prefix-map={path}={replacement}" for path, replacement in prefixes)
env["CFLAGS"] = shlex.join(cflags)
commands = []

def run(name, argv):
    start = datetime.datetime.now(datetime.timezone.utc).isoformat()
    print(name, flush=True)
    with (evidence / (name + '.txt')).open('wb') as log:
        result = subprocess.run([str(v) for v in argv], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    commands.append(dict(name=name, argv=[str(v) for v in argv], started_at=start, exit_code=result.returncode))
    (evidence / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
    if result.returncode:
        raise SystemExit(f'{name} failed ({result.returncode}); see {evidence / (name + ".txt")}')
    return (evidence / (name + '.txt')).read_text(errors='replace')

def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()

identities = run('identities', ['/usr/bin/security', 'find-identity', '-v', '-p', 'codesigning'])
allowed = re.findall(r'([A-F0-9]{40}) "(Developer ID Application: [^"\n]+)"', identities)
if not any(args.identity in pair for pair in allowed):
    raise SystemExit('The selected identity is not an existing valid Developer ID Application identity')
# The CLI first: Tauri copies binaries/kinana-<target triple> into the bundle.
run('cli-build', ['cargo', 'build', '--release', '--locked', '--target', 'aarch64-apple-darwin', '-p', 'kinana', '--bin', 'kinana'])
cli_binary = target / 'aarch64-apple-darwin/release/kinana'
if not (subprocess.check_output(['/usr/bin/lipo', '-archs', cli_binary], text=True).strip() == 'arm64'):
    raise SystemExit("Release verification failed: subprocess.check_output(['/usr/bin/lipo', '-archs', cli_binary], text=True).strip() == 'arm64'")
sidecar = ROOT / 'app/src-tauri/binaries/kinana-aarch64-apple-darwin'
sidecar.parent.mkdir(exist_ok=True)
shutil.copy2(cli_binary, sidecar)
run('native-build', [ROOT / 'scripts/with-node.sh', 'npm', '--prefix', 'app', 'run', 'tauri', '--',
                     'build', '--target', 'aarch64-apple-darwin', '--bundles', 'app',
                     '--config', 'src-tauri/tauri.beta.conf.json', '--no-sign', '--', '--locked'])
app = target / 'aarch64-apple-darwin/release/bundle/macos/Kinana.app'
info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
if not (info['CFBundleIdentifier'] == 'io.github.khaled1174.kinana'):
    raise SystemExit("Release verification failed: info['CFBundleIdentifier'] == 'io.github.khaled1174.kinana'")
if not (info['LSMinimumSystemVersion'] == MINIMUM_MACOS):
    raise SystemExit("Release verification failed: info['LSMinimumSystemVersion'] == MINIMUM_MACOS")
executable = app / 'Contents/MacOS' / info['CFBundleExecutable']
# Exactly two native executables, by name: the app and the CLI beside it.
# Refuse silently signing any newly introduced helper/framework architecture.
bundled_cli = app / 'Contents/MacOS/kinana'
native = []
for p in app.rglob('*'):
    if p.is_file() and not p.is_symlink():
        with p.open('rb') as f:
            if f.read(4) in (b'\xcf\xfa\xed\xfe', b'\xca\xfe\xba\xbe', b'\xfe\xed\xfa\xcf'):
                native.append(p)
if not (sorted(native) == sorted([executable, bundled_cli])):
    raise SystemExit(native)
if not (executable.name == 'kinana-desktop'):
    raise SystemExit(executable)
if not (bundled_cli.read_bytes() == cli_binary.read_bytes()):
    raise SystemExit('the bundled CLI is not the one just built')
for binary in [executable, bundled_cli]:
    if not (subprocess.check_output(['/usr/bin/lipo', '-archs', binary], text=True).strip() == 'arm64'):
        raise SystemExit("Release verification failed: subprocess.check_output(['/usr/bin/lipo', '-archs', binary], text=True).strip() == 'arm64'")
    # Each binary's own load command names the same minimum as Info.plist.
    build_version = subprocess.check_output(['/usr/bin/vtool', '-show-build', binary], text=True)
    if not (re.search(r'minos ' + re.escape(MINIMUM_MACOS) + r'\b', build_version)):
        raise SystemExit(build_version)
if not ((app / 'Contents/Resources/LICENSE').read_bytes() == (ROOT / 'LICENSE').read_bytes()):
    raise SystemExit("Release verification failed: (app / 'Contents/Resources/LICENSE').read_bytes() == (ROOT / 'LICENSE').read_bytes()")
if not ((app / 'Contents/Resources/THIRD_PARTY_NOTICES.md').read_bytes() == (ROOT / 'THIRD_PARTY_NOTICES.md').read_bytes()):
    raise SystemExit("Release verification failed: (app / 'Contents/Resources/THIRD_PARTY_NOTICES.md').read_bytes() == (ROOT / 'THIRD_PARTY_NOTICES.md').read_bytes()")
# Gate signing on the release executable strings.
privacy = {}
for binary in (executable, bundled_cli):
    printable = subprocess.check_output(["/usr/bin/strings", "-a", binary]).lower()
    privacy[binary.name] = {
        "home_matches": printable.count(os.fsencode(home).lower()),
        "username_matches": printable.count(os.fsencode(account.pw_name).lower()),
    }
(evidence / "binary-privacy.json").write_text(json.dumps(privacy, indent=2) + "\n")
if any(count for matches in privacy.values() for count in matches.values()):
    raise SystemExit("Personal path or username in release binary strings; see binary-privacy.json")
# Inside out: the CLI, then the app that seals it.
run('sign-cli', ['/usr/bin/codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', args.identity, bundled_cli])
details = run('signature-cli', ['/usr/bin/codesign', '-dvvv', bundled_cli])
if not ('Developer ID Application:' in details and 'runtime' in details and 'Timestamp=' in details):
    raise SystemExit("Release verification failed: 'Developer ID Application:' in details and 'runtime' in details and 'Timestamp=' in details")
run('sign-app', ['/usr/bin/codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', args.identity, app])
run('verify-app', ['/usr/bin/codesign', '--verify', '--deep', '--strict', '--verbose=4', app])
details = run('signature-app', ['/usr/bin/codesign', '-dvvv', app])
if not ('Developer ID Application:' in details and 'runtime' in details and 'Timestamp=' in details):
    raise SystemExit("Release verification failed: 'Developer ID Application:' in details and 'runtime' in details and 'Timestamp=' in details")
entitlements = run('entitlements-app', ['/usr/bin/codesign', '-d', '--entitlements', '-', app])
if not ('get-task-allow' not in entitlements):
    raise SystemExit("Release verification failed: 'get-task-allow' not in entitlements")
run('verify-cli', ['/usr/bin/codesign', '--verify', '--strict', '--verbose=4', bundled_cli])
run('help-cli', [bundled_cli, '--help'])
version = info['CFBundleShortVersionString']
archives = target / 'artifacts'
archives.mkdir(exist_ok=True)
app_zip = archives / f'Kinana-{version}-beta-arm64.zip'
if not (not app_zip.exists()):
    raise SystemExit('Preserve an existing candidate; choose a fresh build directory')
run('archive-app', ['/usr/bin/ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', app, app_zip])
manifest = {
    'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
    'target': 'aarch64-apple-darwin', 'minimum_macos_candidate': MINIMUM_MACOS,
    'bundle_id': info['CFBundleIdentifier'], 'app': str(app), 'executable': str(executable),
    'developer_id_signed': True, 'hardened_runtime': True, 'secure_timestamp': True,
    'notarization_submitted': False, 'stapled': False, 'published': False,
    'frontend_sha256': {str(p.relative_to(ROOT / 'app/dist')): sha(p) for p in sorted((ROOT / 'app/dist').rglob('*')) if p.is_file()},
    'bundle_sha256': {str(p.relative_to(app)): sha(p) for p in sorted(app.rglob('*')) if p.is_file()},
    'cli': {'bundled': str(bundled_cli), 'sha256': sha(bundled_cli)},
    'archives': {str(p): sha(p) for p in [app_zip]},
}
(evidence / 'artifact.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps(manifest, indent=2), flush=True)
