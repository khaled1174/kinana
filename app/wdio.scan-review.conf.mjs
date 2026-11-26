import {mkdirSync,readFileSync,writeFileSync,appendFileSync} from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {isolatedSession, e2eService, focusTestWindow, prepareTestDriver} from './tests/support/isolated-ui.mjs';
import {syntheticSkill} from './tests/support/synthetic-skill.mjs';

const session=isolatedSession();
const {base,home,evidence,cli,binary}=session;
// The launcher and the worker both load this file; only the first prepares the fixture.
const statePath=path.join(evidence,'state.json');
let state;
if(process.env.KINANA_SCAN_UI_STATE){state=JSON.parse(readFileSync(statePath,'utf8'));}
else{
const fresh=['emil-animations','emil-ask-emil','emil-ask-lapse','emil-borrow-a-brain','emil-build-a-tool','emil-component-design','emil-design-system-docs','emil-design-vocabulary','emil-engineering-vocabulary','emil-forms-and-inputs','emil-get-creative','emil-marketing-pages'];
const root=path.join(base,'مشروع مسح'),skills=path.join(root,'.claude/skills');
for(const id of fresh)syntheticSkill(path.join(skills,id),id);
for(const folder of ['color','color-copy'])syntheticSkill(path.join(skills,folder),'emil-color');
for(const folder of ['design-a','design-b'])syntheticSkill(path.join(skills,folder),'emil-design-foundations');
appendFileSync(path.join(skills,fresh[0],'SKILL.md'),'\nInert example: fetch("https://example.invalid/reference")\n');
appendFileSync(path.join(skills,'design-b/SKILL.md'),'\nInert verification note: a second version of the same name.\n');
mkdirSync(path.join(skills,'broken'));writeFileSync(path.join(skills,'broken/SKILL.md'),'---\nname: broken\n---\n');
const init=spawnSync(cli,['--json','init'],{env:{...process.env,HOME:home},encoding:'utf8'});
if(init.status!==0)throw Error(init.stdout+init.stderr);
state={created_by:'Kinana scan review acceptance (round 16)',base,home,vault:path.join(home,'KinanaVault'),scan_root:root,skills,fresh,e2e_binary:binary,cli,item_delay_ms:300};
writeFileSync(statePath,JSON.stringify(state,null,2)+'\n');
process.env.KINANA_SCAN_UI_STATE=statePath;
}
export const config={
 runner:'local',specs:['./tests/scan-review.e2e.mjs'],maxInstances:1,logLevel:'warn',outputDir:path.join(evidence,'wdio'),framework:'mocha',reporters:['spec'],mochaOpts:{timeout:120000,bail:true},connectionRetryCount:0,
 services:[e2eService(session,{KINANA_E2E_IMPORT_ITEM_DELAY_MS:String(state.item_delay_ms)})],
 capabilities:[{browserName:'tauri'}],
 before:prepareTestDriver,beforeTest:focusTestWindow,
 async afterTest(test,context,{error}){if(error){const name=test.title.replace(/[^a-z0-9]+/gi,'-').slice(0,80);await browser.saveScreenshot(path.join(evidence,`failure-${name}.png`));writeFileSync(path.join(evidence,`failure-${name}.txt`),await browser.execute(()=>document.body.innerText));}},
};
