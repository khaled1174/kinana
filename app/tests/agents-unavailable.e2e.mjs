import assert from 'node:assert/strict';
import {writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
// Case 1 of 2 (owner decision, 2026-11-18): no agent is installed anywhere the
// app looks. Run with launchd's PATH and an empty temporary HOME:
//   KINANA_UI_AGENT_PATH=/usr/bin:/bin:/usr/sbin:/sbin KINANA_UI_AGENT_HOME_CASE=missing \
//   npm run test:ui -- --spec ./tests/agents-unavailable.e2e.mjs
// Case 2, installed outside PATH, is agents-outside-path.e2e.mjs.
const evidence=process.env.KINANA_UI_EVIDENCE||fileURLToPath(new URL('../../docs/evidence/phase-6/batch-4-5/',import.meta.url));
describe('Classifier not installed anywhere the app looks',()=>{
 before(()=>assert.equal(process.env.KINANA_UI_AGENT_HOME_CASE,'missing','run with KINANA_UI_AGENT_HOME_CASE=missing'));
 it('shows unavailable agents and names the cause and the searched places',async()=>{
  await $('[data-testid="nav-settings"]').click();await $('.agent-settings').waitForDisplayed();const bootstrap=await browser.tauri.execute(async({core})=>core.invoke('bootstrap'));assert.equal(bootstrap.agents.length,3);assert.ok(bootstrap.agents.filter(a=>a.name!=='cursor').every(a=>!a.available));await browser.waitUntil(async()=>await $$('.agent-settings article').length===3);assert.match(await $('.agent-settings').getText(),/غير مثبت/);await browser.saveScreenshot(path.join(evidence,'error-agent-unavailable-ar.png'));
  await $('[data-testid="nav-vault"]').click();await $('[data-testid="classify-emil-color"]').waitForEnabled();await $('[data-testid="classify-emil-color"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();await $('[data-testid="confirm-classify"]').click();await $('[data-testid="classification-result"]').waitForDisplayed({timeout:30000});
  const text=await $('[data-testid="classification-result"]').getText();assert.match(text,/تقريبي/);assert.doesNotMatch(text,/CLI|not installed|unavailable|Using keywords|Claude classification rejected/);
  assert.equal(await $('[data-testid="classification-reason"]').getAttribute('data-reason'),'AGENT_EXECUTABLE_NOT_FOUND');
  const searched=await $('[data-testid="classification-searched"]').getText();assert.match(searched,/\.local\/bin \(not present\)/);assert.match(searched,/\/usr\/bin/);
  const detail=await browser.tauri.execute(async({core})=>core.invoke('skill_detail',{id:'emil-color'}));assert.equal(detail.detail.metadata.classification.classified_by,'keywords');assert.equal(detail.detail.metadata.proposal,undefined);
  writeFileSync(path.join(evidence,'agent-unavailable.json'),JSON.stringify({restricted_path:true,agent_home:'missing',bootstrap,visible:text,searched,classified_by:'keywords'},null,2)+'\n');
 });
});
