import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const app = fileURLToPath(new URL('..', import.meta.url));
const e2e = process.argv[2] === 'e2e';
const target = (e2e && process.env.CARGO_TARGET_DIR) || path.resolve(app, `../target/phase6-${e2e ? 'e2e' : 'release'}`);
const args = ['run', 'tauri', '--', 'build'];
if (e2e) {
  if (existsSync(path.join(app, 'node_modules/node_modules/react'))) {
    throw new Error('Duplicate React in a nested node_modules copy. Install dependencies into an empty app/node_modules.');
  }
  args.push('--bundles', 'app', '--features', 'e2e', '--config', 'src-tauri/tauri.e2e.conf.json');
} else {
  args.push('--no-bundle');
}
args.push('--', '--locked');
console.log(`Building ${e2e ? 'e2e (feature e2e)' : 'default release (no e2e feature)'} in ${target}`);
const env = { ...process.env, CARGO_TARGET_DIR: target };
if (e2e) {
  for (const key of Object.keys(env)) {
    if (key.startsWith('APPLE_')) delete env[key];
  }
}
const result = spawnSync('npm', args, { cwd: app, env, stdio: 'inherit' });
if (result.error) throw result.error;
if (result.status) process.exit(result.status);
// The isolated runner takes the CLI from the target alongside the bundle.
const cli = spawnSync('cargo', ['build', '--release', '--locked', '-p', 'kinana', '--bin', 'kinana'], { cwd: app, env, stdio: 'inherit' });
if (cli.error) throw cli.error;
process.exit(cli.status ?? 1);
