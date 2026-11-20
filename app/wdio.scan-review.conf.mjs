import {cpSync,mkdirSync,mkdtempSync,readFileSync,realpathSync,writeFileSync,appendFileSync} from 'node:fs';
import path from 'node:path';
import {tmpdir} from 'node:os';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
// Scan review acceptance (round 16): the owner's first run in small, in an isolated HOME.
// The test copies repository fixtures and calls no agent.
const app=fileURLToPath(new URL('.',import.meta.url));
const evidence=process.env.KINANA_UI_EVIDENCE;
if(!evidence||!process.env.KINANA_E2E_BINARY||!process.env.KINANA_TEST_CLI)throw Error('Set isolated evidence, e2e binary, and test CLI paths.');
mkdirSync(evidence,{recursive:true});
// The launcher and the worker both load this file; only the first prepares the fixture.
const statePath=path.join(evidence,'state.json');
let state;
if(process.env.KINANA_SCAN_UI_STATE){state=JSON.parse(readFileSync(statePath,'utf8'));}
else{
const base=mkdtempSync(path.join(realpathSync(tmpdir()),'kinana-scan-ui-'));
const home=path.join(base,'home');mkdirSync(home);
const installed=path.join(app,'..','tests','fixtures','public-release','skills');
const fresh=['emil-animations','emil-ask-emil','emil-ask-lapse','emil-borrow-a-brain','emil-build-a-tool','emil-component-design','emil-design-system-docs','emil-design-vocabulary','emil-engineering-vocabulary','emil-forms-and-inputs','emil-get-creative','emil-marketing-pages'];
const root=path.join(base,'مشروع مسح'),skills=path.join(root,'.claude/skills');
for(const id of fresh)cpSync(path.join(installed,id),path.join(skills,id),{recursive:true});
cpSync(path.join(installed,'emil-color'),path.join(skills,'color'),{recursive:true});
cpSync(path.join(installed,'emil-color'),path.join(skills,'color-copy'),{recursive:true});
cpSync(path.join(installed,'emil-design-foundations'),path.join(skills,'design-a'),{recursive:true});
cpSync(path.join(installed,'emil-design-foundations'),path.join(skills,'design-b'),{recursive:true});
appendFileSync(path.join(skills,'design-b/SKILL.md'),'\nInert verification note: a second version of the same name.\n');
mkdirSync(path.join(skills,'broken'));writeFileSync(path.join(skills,'broken/SKILL.md'),'---\nname: broken\n---\n');
const init=spawnSync(process.env.KINANA_TEST_CLI,['--json','init'],{env:{...process.env,HOME:home},encoding:'utf8'});
if(init.status!==0)throw Error(init.stdout+init.stderr);
state={created_by:'Kinana scan review acceptance (round 16)',base,home,vault:path.join(home,'KinanaVault'),scan_root:root,skills,installed,fresh,e2e_binary:process.env.KINANA_E2E_BINARY,cli:process.env.KINANA_TEST_CLI,item_delay_ms:300};
writeFileSync(statePath,JSON.stringify(state,null,2)+'\n');
process.env.KINANA_SCAN_UI_STATE=statePath;
}
const {home}=state;
export const config={
 runner:'local',specs:['./tests/scan-review.e2e.mjs'],maxInstances:1,logLevel:'warn',outputDir:path.join(evidence,'wdio'),framework:'mocha',reporters:['spec'],mochaOpts:{timeout:120000,bail:true},connectionRetryCount:0,
 services:[['@wdio/tauri-service',{driverProvider:'embedded',appBinaryPath:process.env.KINANA_E2E_BINARY,env:{HOME:process.env.HOME,KINANA_E2E_HOME:home,KINANA_E2E_IMPORT_ITEM_DELAY_MS:String(state.item_delay_ms),PATH:process.env.PATH},captureBackendLogs:true,captureFrontendLogs:true}]],
 capabilities:[{browserName:'tauri'}],
 async beforeTest(){await browser.tauri.execute(async({core})=>core.invoke('plugin:window|set_focus',{label:'main'}));await browser.waitUntil(async()=>await browser.execute(()=>document.visibilityState==='visible'));},
 async afterTest(test,context,{error}){if(error){const name=test.title.replace(/[^a-z0-9]+/gi,'-').slice(0,80);await browser.saveScreenshot(path.join(evidence,`failure-${name}.png`));writeFileSync(path.join(evidence,`failure-${name}.txt`),await browser.execute(()=>document.body.innerText));}},
};
