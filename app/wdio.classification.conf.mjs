import {cpSync,mkdirSync,mkdtempSync,readFileSync,realpathSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath} from 'node:url';
import {classificationFixture} from './tests/support/classification-fixture.mjs';
const app=fileURLToPath(new URL('.',import.meta.url)),repo=path.resolve(app,'..');
const evidence=process.env.KINANA_UI_EVIDENCE;
if(!evidence||!process.env.KINANA_E2E_BINARY||!process.env.KINANA_TEST_CLI)throw Error('Set isolated evidence, e2e binary, and test CLI paths.');
mkdirSync(evidence,{recursive:true});
const base=process.env.KINANA_UI_BASE||mkdtempSync(path.join(realpathSync(tmpdir()),'kinana-batch-ui-'));
const home=process.env.KINANA_UI_HOME||path.join(base,'home');mkdirSync(home,{recursive:true});
// Synthetic skills are copied into the isolated agent home once.
const seeded=path.join(base,'agent-skills-seeded.json');
if(!process.env.KINANA_UI_BASE){
 for(const id of ['emil-color','emil-design-foundations'])cpSync(path.join(path.join(repo,'tests','fixtures','public-release','skills'),id),path.join(home,'.codex/skills',id),{recursive:true});
 writeFileSync(seeded,JSON.stringify({source:path.join(repo,'tests','fixtures','public-release','skills'),skills:['emil-color','emil-design-foundations']})+'\n');
}
const fixturePath=path.join(base,'batch-provider-fixture.json');
const fixture=process.env.KINANA_CLASSIFY_BIN?JSON.parse(readFileSync(fixturePath,'utf8')):classificationFixture(base,repo);
writeFileSync(fixturePath,JSON.stringify(fixture,null,2)+'\n');
Object.assign(process.env,{KINANA_UI_BASE:base,KINANA_UI_HOME:home,KINANA_CLASSIFY_TRACE:fixture.trace,KINANA_CLASSIFY_HOLD:fixture.hold,KINANA_CLASSIFY_BIN:fixture.bin});
writeFileSync(path.join(evidence,'state.json'),JSON.stringify({created_by:'Kinana native batch classification acceptance',base,home,fixture,e2e_binary:process.env.KINANA_E2E_BINARY,cli:process.env.KINANA_TEST_CLI},null,2)+'\n');
export const config={
 runner:'local',specs:['./tests/classification.e2e.mjs'],maxInstances:1,logLevel:'warn',outputDir:path.join(evidence,'wdio'),framework:'mocha',reporters:['spec'],mochaOpts:{timeout:90000,bail:true},connectionRetryCount:0,
 services:[['@wdio/tauri-service',{driverProvider:'embedded',appBinaryPath:process.env.KINANA_E2E_BINARY,env:{HOME:process.env.HOME,KINANA_E2E_HOME:home,PATH:fixture.bin+path.delimiter+process.env.PATH},captureBackendLogs:true,captureFrontendLogs:true}]],
 capabilities:[{browserName:'tauri'}],
 async beforeTest(){await browser.tauri.execute(async({core})=>core.invoke('plugin:window|set_focus',{label:'main'}));await browser.waitUntil(async()=>await browser.execute(()=>document.visibilityState==='visible'));},
 async afterTest(test,context,{error}){if(error){await browser.saveScreenshot(path.join(evidence,'failure.png'));const text=await browser.execute(()=>document.body.innerText);writeFileSync(path.join(evidence,'failure-ui.txt'),text);}},
};
