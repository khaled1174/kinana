// Live "classify all" acceptance through the native app and core, with the
// real `claude` CLI on the owner's account. Runs only with KINANA_LIVE_AGENT=1
// and KINANA_LIVE_MAX_CALLS (owner permission, round 9: 3 skills, 3 calls
// expected, at most 6). App data lives in an isolated temporary HOME; the
// provider keeps the real HOME for its own authentication only.
import {cpSync,mkdirSync,mkdtempSync,realpathSync,writeFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {tmpdir} from 'node:os';
const evidence=process.env.KINANA_UI_EVIDENCE,cli=process.env.KINANA_TEST_CLI;
if(process.env.KINANA_LIVE_AGENT!=='1'||!process.env.KINANA_LIVE_MAX_CALLS)throw Error('Live classification needs KINANA_LIVE_AGENT=1 and KINANA_LIVE_MAX_CALLS (owner permission).');
if(!evidence||!process.env.KINANA_E2E_BINARY||!cli)throw Error('Set isolated evidence, e2e binary, and test CLI paths.');
mkdirSync(evidence,{recursive:true});
// WDIO evaluates this file in the launcher and again in the worker: create
// the vault once and let the worker reuse it through the environment.
const base=process.env.KINANA_UI_BASE||mkdtempSync(path.join(realpathSync(tmpdir()),'kinana-live-all-'));
const fresh=!process.env.KINANA_UI_BASE;
const home=path.join(base,'home'),vault=path.join(home,'KinanaVault');mkdirSync(home,{recursive:true});
const ids=['emil-color','emil-design-foundations','emil-forms-and-inputs'];
function run(args){const p=spawnSync(cli,['--vault',vault,'--json',...args],{env:{...process.env,HOME:home},encoding:'utf8'});if(p.status)throw Error(p.stdout+p.stderr);return JSON.parse(p.stdout);}
if(fresh){run(['init']);
for(const id of ids){const copy=path.join(base,'sources',id);cpSync(path.join(process.env.HOME,'.codex/skills',id),copy,{recursive:true});run(['add',copy]);}}
Object.assign(process.env,{KINANA_UI_BASE:base,KINANA_UI_HOME:home,KINANA_LIVE_VAULT:vault,KINANA_LIVE_IDS:ids.join(',')});
if(fresh)writeFileSync(path.join(evidence,'state.json'),JSON.stringify({created_by:'Kinana live classify-all acceptance',base,home,vault,skills:ids,max_calls:Number(process.env.KINANA_LIVE_MAX_CALLS),e2e_binary:process.env.KINANA_E2E_BINARY,cli},null,2)+'\n');
export const config={
 runner:'local',specs:['./tests/classification-live.e2e.mjs'],maxInstances:1,logLevel:'warn',outputDir:path.join(evidence,'wdio'),framework:'mocha',reporters:['spec'],mochaOpts:{timeout:600000,bail:true},connectionRetryCount:0,
 services:[['@wdio/tauri-service',{driverProvider:'embedded',appBinaryPath:process.env.KINANA_E2E_BINARY,env:{HOME:process.env.HOME,KINANA_E2E_HOME:home,PATH:process.env.PATH},captureBackendLogs:true,captureFrontendLogs:true}]],
 capabilities:[{browserName:'tauri'}],
};
