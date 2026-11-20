import {agentHome} from './tests/support/agent-home.mjs';
import { readFileSync, writeFileSync, mkdirSync, realpathSync, mkdtempSync, renameSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import {prepareSourceFixture} from './scripts/source-ui-fixture.mjs';
const app=fileURLToPath(new URL('.',import.meta.url));
const evidence=process.env.KINANA_UI_EVIDENCE||path.resolve(app,'../docs/evidence/phase-6/batch-4-5');mkdirSync(evidence,{recursive:true});
const fixture=JSON.parse(readFileSync(path.resolve(app,'../.git/phase62-live-fixture.json'),'utf8'));
let home=process.env.KINANA_UI_HOME;
let base=process.env.KINANA_UI_BASE;
if (!home) {
 base=mkdtempSync(path.join(realpathSync(tmpdir()),'kinana-phase62-ui-'));home=path.join(base,'home');mkdirSync(home);
 const vault=path.join(home,'SkillsBox');
 if(!process.env.KINANA_UI_EMPTY){
 const env={...process.env,HOME:home,GIT_CONFIG_NOSYSTEM:'1',GIT_CONFIG_GLOBAL:'/dev/null',GIT_ATTR_NOSYSTEM:'1'};
 function run(command,args){const p=spawnSync(command,args,{env,encoding:'utf8'});if(p.status!==0)throw new Error(p.stdout+p.stderr);return p.stdout;}
 run('/usr/bin/git',['-c','core.hooksPath=/dev/null','clone','--quiet','--no-hardlinks','--no-checkout','--template=','--',fixture.vault,vault]);
 run('/usr/bin/git',['-C',vault,'-c','core.hooksPath=/dev/null','checkout','--quiet','--detach',fixture.baseline_commit]);
 for(const dir of ['skills','meta','bundles','dist'])mkdirSync(path.join(vault,dir),{recursive:true});
 const cli=path.resolve(app,'../target/debug/kinana');
 run(cli,['--vault',vault,'--json','doctor','--rebuild-index']);
 run(cli,['--vault',vault,'--json','build']);
 if(process.env.KINANA_UI_LEGACY)renameSync(path.join(home,'.kinana'),path.join(home,'.skillsbox'));
 writeFileSync(path.join(evidence,'ui-fixture-provenance.json'),JSON.stringify({home,vault,source_vault:fixture.vault,source_commit:fixture.baseline_commit,sources:fixture.sources,method:'fresh clone of the real imported vault with live Claude proposal'},null,2)+'\n');
 }
}
if (!realpathSync(home).startsWith(realpathSync(tmpdir())+path.sep) || !path.basename(base).startsWith('kinana-phase62-')) throw new Error('Native UI tests require an isolated test vault; see app/README.md');
process.env.KINANA_UI_HOME=home;process.env.KINANA_UI_BASE=base;
const sourceFixture=process.env.KINANA_UI_SOURCE_FIXTURE?JSON.parse(readFileSync(process.env.KINANA_UI_SOURCE_FIXTURE,'utf8')):prepareSourceFixture(base);process.env.KINANA_UI_SOURCE_FIXTURE=path.join(sourceFixture.root,'fixture.json');process.env.KINANA_UI_AGENT_PATH=sourceFixture.bin+path.delimiter+(process.env.KINANA_UI_AGENT_PATH||process.env.PATH);
writeFileSync(path.join(evidence,'source-ui-fixture.json'),JSON.stringify(sourceFixture,null,2)+'\n');
mkdirSync(path.join(app,'test-results'),{recursive:true});
// KINANA_UI_AGENT_HOME_CASE=missing|outside-path: the app gets a temporary HOME too, so the
// owner's real ~/.local/bin is never searched (use with KINANA_UI_AGENT_PATH=launchd's PATH).
const agentCase=process.env.KINANA_UI_AGENT_HOME_CASE?agentHome(base,path.resolve(app,'..'),process.env.KINANA_UI_AGENT_HOME_CASE):null;
if(agentCase)writeFileSync(path.join(evidence,`agent-home-${process.env.KINANA_UI_AGENT_HOME_CASE}.json`),JSON.stringify(agentCase,null,2)+'\n');
export const config={
 runner:'local',specs:['./tests/shell.e2e.mjs','./tests/vault.e2e.mjs','./tests/projects.e2e.mjs','./tests/settings.e2e.mjs','./tests/acceptance.e2e.mjs','./tests/cursor.e2e.mjs','./tests/source-updates.e2e.mjs','./tests/vault-location.e2e.mjs'],maxInstances:1,
 logLevel:'warn',outputDir:'./test-results',framework:'mocha',reporters:['spec'],
 async beforeTest(){await browser.tauri.execute(async({core})=>core.invoke('plugin:window|set_focus',{label:'main'}));await browser.waitUntil(async()=>await browser.execute(()=>document.visibilityState==='visible'));},
 mochaOpts:{timeout:180000},connectionRetryCount:0,
 services:[['@wdio/tauri-service',{driverProvider:'embedded',appBinaryPath:path.resolve(app,'../target/phase6-e2e/release/kinana-desktop'),env:{HOME:agentCase?agentCase.home:process.env.HOME,KINANA_E2E_HOME:home,...(process.env.KINANA_UI_AGENT_PATH?{PATH:process.env.KINANA_UI_AGENT_PATH}:{})},captureBackendLogs:true,captureFrontendLogs:true}]],
 capabilities:[{browserName:'tauri'}],
 async afterTest(test,context,{error}){if(error){const name=test.title.replace(/[^a-z0-9]+/gi,'-').slice(0,80);await browser.saveScreenshot(path.resolve(app,'test-results',name+'.png'));const dom=await browser.execute(()=>({errors:Array.from(document.querySelectorAll('[role="alert"]')).map(e=>e.textContent),dialogs:Array.from(document.querySelectorAll('[role="dialog"]')).map(e=>e.textContent),search:document.querySelector('[data-testid="search"]')?.value,visibility:document.visibilityState,styles:Array.from(document.querySelectorAll('[data-testid="classify-estimate"],[data-slot="dialog-content"],[data-slot="dialog-overlay"]')).map(e=>{const c=getComputedStyle(e),r=e.getBoundingClientRect();return{slot:e.dataset.slot,testid:e.dataset.testid,opacity:c.opacity,display:c.display,visibility:c.visibility,animation:c.animation,rect:{x:r.x,y:r.y,width:r.width,height:r.height}}})}));writeFileSync(path.resolve(app,'test-results',name+'.json'),JSON.stringify(dom,null,2));}},
 onComplete(){const result=path.join(evidence,'onboarding-result.json');const vault=process.env.KINANA_UI_EMPTY?(existsSync(result)?JSON.parse(readFileSync(result,'utf8')).vault:null):path.join(home,'SkillsBox');writeFileSync(path.join(evidence,process.env.KINANA_UI_EMPTY?'onboarding-session.json':'ui-session.json'),JSON.stringify({home,vault,native:true,mocked:false},null,2)+'\n');},
};
