import {mkdirSync,copyFileSync,chmodSync} from 'node:fs';
import path from 'node:path';
import {classificationFixture} from './classification-fixture.mjs';
// The HOME an app opened from Finder sees, for the two agent-location cases
// (owner decision, 2026-11-18). "missing": no agent anywhere the app looks.
// "outside-path": a fake Claude (no model call) only in ~/.local/bin, where
// the official installer puts it and launchd's PATH does not reach.
export function agentHome(base,repo,kind){
 const home=path.join(base,`agent-home-${kind}`);mkdirSync(home,{recursive:true,mode:0o755});
 if(kind==='outside-path'){
  const fixture=classificationFixture(base,repo),bin=path.join(home,'.local/bin');
  mkdirSync(bin,{recursive:true,mode:0o755});copyFileSync(path.join(fixture.bin,'claude'),path.join(bin,'claude'));chmodSync(path.join(bin,'claude'),0o755);
  return {home,trace:fixture.trace};
 }
 if(kind!=='missing')throw Error(`Unknown agent home case: ${kind}`);
 return {home,trace:null};
}
