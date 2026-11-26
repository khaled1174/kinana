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
 it('shows the stopped provider and offers an explicit keywords action',async()=>{
  await $('[data-testid="nav-settings"]').click();await $('.agent-settings').waitForDisplayed();const bootstrap=await browser.tauri.execute(async({core})=>core.invoke('bootstrap'));assert.equal(bootstrap.agents.length,3);assert.ok(bootstrap.agents.filter(a=>a.name!=='cursor').every(a=>!a.available));await browser.waitUntil(async()=>await $$('.agent-settings article').length===3);assert.match(await $('.agent-settings').getText(),/غير مثبت/);await browser.saveScreenshot(path.join(evidence,'error-agent-unavailable-ar.png'));
  const skill=()=>browser.tauri.execute(async({core})=>core.invoke('skill_detail',{id:'emil-color'}));
  const before=(await skill()).detail.metadata;
  await $('[data-testid="nav-vault"]').click();await $('[data-testid="skill-emil-color"] .skill-select').click();await $('[data-testid="detail-classify"]').waitForEnabled();await $('[data-testid="detail-classify"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();await $('[data-testid="confirm-classify"]').click();
  const stopped=$('[data-testid="classification-stopped"]');await stopped.waitForDisplayed({timeout:30000});
  const reason=await stopped.getAttribute('data-reason');assert.equal(reason,'AGENT_AUTO_UNAVAILABLE');
  const provider=$('[data-testid="classification-providers"] [data-agent="claude"]');await provider.waitForDisplayed();
  const text=await stopped.getText();assert.match(await provider.getText(),/Claude/);assert.deepEqual((await skill()).detail.metadata,before);
  await $('[data-testid="classify-with-keywords"]').click();await $('[data-testid="confirm-classify"]').waitForEnabled();await $('[data-testid="confirm-classify"]').click();
  await $('[data-testid="classification-result"]').waitForDisplayed();
  const detail=await skill();assert.equal(detail.detail.metadata.classification.classified_by,'keywords');assert.equal(detail.detail.metadata.proposal,undefined);
  writeFileSync(path.join(evidence,'agent-unavailable.json'),JSON.stringify({restricted_path:true,agent_home:'missing',bootstrap,visible:text,reason,unchanged_until_explicit_keywords:true,classified_by:'keywords'},null,2)+'\n');
 });
});
