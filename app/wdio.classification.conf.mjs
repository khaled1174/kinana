import {readFileSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {isolatedSession, e2eService, focusTestWindow, prepareTestDriver} from './tests/support/isolated-ui.mjs';
import {syntheticSkill} from './tests/support/synthetic-skill.mjs';
import {classificationFixture} from './tests/support/classification-fixture.mjs';
import {piFixture} from './tests/support/pi-fixture.mjs';
const app=fileURLToPath(new URL('.',import.meta.url)),repo=path.resolve(app,'..');
const session=isolatedSession();
const {base,home,evidence}=session;
if(!process.env.KINANA_CLASSIFY_BIN){
 for(const id of ['emil-color','emil-design-foundations'])syntheticSkill(path.join(home,'.codex/skills',id),id);
}
const fixturePath=path.join(base,'batch-provider-fixture.json');
const fixture=process.env.KINANA_CLASSIFY_BIN?JSON.parse(readFileSync(fixturePath,'utf8')):classificationFixture(base,repo);
writeFileSync(fixturePath,JSON.stringify(fixture,null,2)+'\n');
// r5: a fake Pi 1.5.2 next to the fake Claude; only the e2e build accepts its manifest.
const pi=piFixture(base,fixture.bin);
Object.assign(process.env,{KINANA_FAKE_PI_DIR:pi.dir,KINANA_TEST_PI_PACKAGE_MANIFEST:pi.manifest,KINANA_PI_TRACE:pi.trace,KINANA_PI_LIST_MODE:pi.listMode,KINANA_CLASSIFY_BROKEN:fixture.broken??path.join(base,'broken-classification'),KINANA_UI_BASE:base,KINANA_UI_HOME:home,KINANA_CLASSIFY_TRACE:fixture.trace,KINANA_CLASSIFY_HOLD:fixture.hold,KINANA_CLASSIFY_SILENT:fixture.silent??path.join(base,'silent-classification'),KINANA_CLASSIFY_BIN:fixture.bin});
writeFileSync(path.join(evidence,'state.json'),JSON.stringify({created_by:'Kinana native batch classification acceptance',base,home,fixture,e2e_binary:process.env.KINANA_E2E_BINARY,cli:process.env.KINANA_TEST_CLI},null,2)+'\n');
export const config={
 runner:'local',specs:['./tests/classification.e2e.mjs'],maxInstances:1,logLevel:'warn',outputDir:path.join(evidence,'wdio'),framework:'mocha',reporters:['spec'],mochaOpts:{timeout:90000,bail:true},connectionRetryCount:0,
 services:[e2eService(session,{PATH:fixture.bin+path.delimiter+process.env.PATH})],
 capabilities:[{browserName:'tauri'}],
 before:prepareTestDriver,beforeTest:focusTestWindow,
 async afterTest(test,context,{error}){if(error){await browser.saveScreenshot(path.join(evidence,'failure.png'));const text=await browser.execute(()=>document.body.innerText);writeFileSync(path.join(evidence,'failure-ui.txt'),text);}},
};
