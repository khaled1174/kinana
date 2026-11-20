#!/usr/bin/env python3
"""Notarize, staple and verify a signed beta candidate. Never publishes.

  notarize-macos.py app --artifact <signed-build/artifact.json> --target-dir <fresh dir> --evidence <dir>
  notarize-macos.py dmg --package <dmg/package.json> --evidence <dir>

`app` submits the signed app archive made by build-beta-macos.py, staples the
ticket to a copy of the signed app, archives and unpacks that copy, and writes
final-artifact.json (the input package-dmg-macos.py --manifest expects). The
signed app is never modified or re-signed; only the ticket is added.

`dmg` submits the signed DMG made by package-dmg-macos.py, staples it, then
mounts it read-only and checks the app inside byte for byte against the
notarized app (every file, ticket included), its signature, ticket and
Gatekeeper assessment, and the Applications drop link.

Credentials come only from the existing notarytool keychain profile."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PROFILE = 'kinana-notary'
parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
sub = parser.add_subparsers(dest='mode', required=True)
p_app = sub.add_parser('app')
p_app.add_argument('--artifact', type=Path, required=True)
p_app.add_argument('--target-dir', type=Path, required=True)
p_app.add_argument('--evidence', type=Path, required=True)
p_app.add_argument('--accepted-id', help='Use an earlier Accepted submission of exactly this archive (its notarytool log must name the same SHA-256) instead of submitting again')
p_dmg = sub.add_parser('dmg')
p_dmg.add_argument('--package', type=Path, required=True)
p_dmg.add_argument('--evidence', type=Path, required=True)
args = parser.parse_args()
evidence = args.evidence.resolve()
evidence.mkdir(parents=True, exist_ok=True)
env = os.environ.copy()
for key in list(env):
    if key.startswith(('APPLE_', 'TAURI_SIGNING_')):
        del env[key]
commands = []


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def run(name, argv, check=True):
    print(name, flush=True)
    start = now()
    p = subprocess.run([str(v) for v in argv], cwd=ROOT, env=env, capture_output=True)
    (evidence / f'{name}.stdout.txt').write_bytes(p.stdout)
    (evidence / f'{name}.stderr.txt').write_bytes(p.stderr)
    commands.append(dict(name=name, argv=[str(v) for v in argv], started_at=start, exit_code=p.returncode))
    (evidence / f'{args.mode}-commands.json').write_text(json.dumps(commands, indent=2) + '\n')
    if check and p.returncode:
        raise SystemExit(f'{name} failed ({p.returncode}); see {evidence / (name + ".stderr.txt")}')
    return p


def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def files(app):
    if any(p.is_symlink() for p in app.rglob('*')):
        raise SystemExit(f'Unexpected links in {app}')
    return {str(p.relative_to(app)): sha(p) for p in sorted(app.rglob('*')) if p.is_file()}


def submit(name, path):
    """One submission, waited on; no retry. Accepted is required."""
    p = run(name, ['xcrun', 'notarytool', 'submit', path, '--keychain-profile', PROFILE,
                   '--wait', '--output-format', 'json'], check=False)
    result = json.loads(p.stdout or b'{}')
    job = result.get('id')
    if job:
        run(f'{name}-log', ['xcrun', 'notarytool', 'log', job, '--keychain-profile', PROFILE,
                            evidence / f'{name}-log.json'], check=False)
    if p.returncode or result.get('status') != 'Accepted':
        raise SystemExit(f'{name}: notarization not accepted ({result.get("status")}); see {evidence}')
    return {'id': job, 'status': result['status'], 'input_sha256': sha(path)}


def reuse(name, path, job):
    """An earlier submission, only if Apple accepted this exact archive."""
    info = run(f'{name}-info', ['xcrun', 'notarytool', 'info', job, '--keychain-profile', PROFILE,
                                '--output-format', 'json'])
    run(f'{name}-log', ['xcrun', 'notarytool', 'log', job, '--keychain-profile', PROFILE,
                        evidence / f'{name}-log.json'])
    status = json.loads(info.stdout).get('status')
    log = json.loads((evidence / f'{name}-log.json').read_text())
    digest = sha(path)
    if status != 'Accepted' or log.get('status') != 'Accepted' or log.get('sha256') != digest \
            or log.get('archiveFilename') != path.name or log.get('issues'):
        raise SystemExit(f'{name}: submission {job} is not an accepted submission of {path} ({digest})')
    return {'id': job, 'status': status, 'input_sha256': digest, 'reused_submission': True}


if args.mode == 'app':
    built = json.loads(args.artifact.read_text())
    signed_app = Path(built['app'])
    signed_zip = Path(next(iter(built['archives'])))
    if files(signed_app) != built['bundle_sha256']:
        raise SystemExit('The signed app differs from its build manifest')
    if sha(signed_zip) != built['archives'][str(signed_zip)]:
        raise SystemExit('The signed archive differs from its build manifest')
    target = args.target_dir.resolve()
    if target.exists():
        parser.error('An existing notarized candidate is preserved; choose a fresh --target-dir')
    (target / 'work').mkdir(parents=True)
    (target / 'artifacts').mkdir()
    job = reuse('notarize-app', signed_zip, args.accepted_id) if args.accepted_id else submit('notarize-app', signed_zip)
    work = target / 'work/Kinana.app'
    run('copy-signed-app', ['/usr/bin/ditto', signed_app, work])
    run('staple-app', ['xcrun', 'stapler', 'staple', '-v', work])
    run('validate-stapled-app', ['xcrun', 'stapler', 'validate', '-v', work])
    stapled = files(work)
    added = sorted(set(stapled) - set(built['bundle_sha256']))
    changed = sorted(k for k in built['bundle_sha256'] if stapled.get(k) != built['bundle_sha256'][k])
    if added != ['Contents/CodeResources'] or changed:
        raise SystemExit(f'Stapling changed more than the ticket: added={added} changed={changed}')
    info = plistlib.loads((work / 'Contents/Info.plist').read_bytes())
    archive = target / 'artifacts' / f"Kinana-{info['CFBundleShortVersionString']}-beta-arm64.zip"
    run('archive-stapled-app', ['/usr/bin/ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', work, archive])
    unpacked = target / 'unpacked/app'
    run('unpack-final-app', ['/usr/bin/ditto', '-x', '-k', archive, unpacked])
    app = unpacked / 'Kinana.app'
    if files(app) != stapled:
        raise SystemExit('The unpacked archive differs from the stapled app')
    run('app-stapler-validate', ['xcrun', 'stapler', 'validate', '-v', app])
    run('app-codesign', ['/usr/bin/codesign', '--verify', '--deep', '--strict', '--verbose=4', app])
    run('app-spctl', ['/usr/sbin/spctl', '--assess', '--type', 'execute', '-vv', app])
    cli = app / 'Contents/MacOS/kinana'
    run('cli-codesign', ['/usr/bin/codesign', '--verify', '--strict', '--verbose=4', cli])
    run('cli-help', [cli, '--version'])
    manifest = {
        'created_at': now(), 'source_build_revision': built['revision'], 'profile': PROFILE,
        'notary_jobs': {'app': job}, 'app': str(app), 'executable': str(app / 'Contents/MacOS/kinana-desktop'),
        'cli': {'bundled': str(cli), 'sha256': sha(cli), 'same_as_signed_build': sha(cli) == built['cli']['sha256']},
        'stapled': True, 'bundle_sha256': stapled, 'archives': {str(archive): sha(archive)},
        'signed_build_manifest': str(args.artifact.resolve()), 'published': False,
    }
    if not manifest['cli']['same_as_signed_build']:
        raise SystemExit('The bundled CLI changed after signing')
    (evidence / 'final-artifact.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({k: v for k, v in manifest.items() if k != 'bundle_sha256'}, indent=2), flush=True)
else:
    package = json.loads(args.package.read_text())
    dmg = Path(package['dmg'])
    if sha(dmg) != package['signed_dmg_sha256_before_notarization']:
        raise SystemExit('The signed DMG differs from its packaging manifest')
    job = submit('notarize-dmg', dmg)
    run('staple-dmg', ['xcrun', 'stapler', 'staple', dmg])
    run('validate-dmg', ['xcrun', 'stapler', 'validate', dmg])
    run('dmg-spctl', ['/usr/sbin/spctl', '-a', '-t', 'open', '--context', 'context:primary-signature', '-vv', dmg])
    run('dmg-codesign', ['/usr/bin/codesign', '--verify', '--strict', '--verbose=4', dmg])
    run('dmg-integrity', ['/usr/bin/hdiutil', 'verify', dmg])
    with tempfile.TemporaryDirectory(dir=evidence.parent) as mount:
        run('attach-dmg', ['/usr/bin/hdiutil', 'attach', '-readonly', '-nobrowse', '-mountpoint', mount, dmg])
        try:
            mounted = Path(mount) / 'Kinana.app'
            same = files(mounted) == package['bundle_file_sha256']
            link = Path(mount) / 'Applications'
            link_ok = link.is_symlink() and os.readlink(link) == '/Applications'
            run('mounted-app-codesign', ['/usr/bin/codesign', '--verify', '--deep', '--strict', '--verbose=4', mounted])
            run('mounted-app-ticket', ['xcrun', 'stapler', 'validate', mounted])
            run('mounted-app-spctl', ['/usr/sbin/spctl', '--assess', '--type', 'execute', '-vv', mounted])
        finally:
            run('detach-dmg', ['/usr/bin/hdiutil', 'detach', mount])
    if not same or not link_ok:
        raise SystemExit(f'Mounted DMG check failed: app identical={same}, Applications link={link_ok}')
    result = {'checked_at': now(), 'dmg': str(dmg), 'notary_job': job, 'stapled_dmg_sha256': sha(dmg),
              'mounted_app_identical_to_notarized': same, 'applications_link': link_ok, 'published': False}
    (evidence / 'verification.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2), flush=True)
