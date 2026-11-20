import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const app = fileURLToPath(new URL('..', import.meta.url));
const e2e = process.argv[2] === 'e2e';
const target = path.resolve(app, `../target/phase6-${e2e ? 'e2e' : 'release'}`);
const args = ['run', 'tauri', '--', 'build', '--no-bundle'];
if (e2e) args.push('--features', 'e2e', '--config', 'src-tauri/tauri.e2e.conf.json');
args.push('--', '--locked');
console.log(`Building ${e2e ? 'e2e (feature e2e)' : 'default release (no e2e feature)'} in ${target}`);
const env = { ...process.env, CARGO_TARGET_DIR: target };
const result = spawnSync('npm', args, { cwd: app, env, stdio: 'inherit' });
if (result.error) throw result.error;
if (result.status) process.exit(result.status);
// The router names the `kinana` beside the app's executable (Contents/MacOS
// in the bundle); these unbundled builds put it in the same folder.
const cli = spawnSync('cargo', ['build', '--release', '--locked', '-p', 'kinana', '--bin', 'kinana'], { cwd: app, env, stdio: 'inherit' });
if (cli.error) throw cli.error;
process.exit(cli.status ?? 1);
