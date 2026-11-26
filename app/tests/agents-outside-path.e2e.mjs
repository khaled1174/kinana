import assert from 'node:assert/strict';
import {readFileSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
// Case 2 of 2 (owner decision, 2026-11-18): Claude is installed in ~/.local/bin,
// which launchd's PATH (an app opened from Finder) does not include. The app
// must find it there. The Claude here is a fake: no model call is made.
//   KINANA_UI_AGENT_PATH=/usr/bin:/bin:/usr/sbin:/sbin KINANA_UI_AGENT_HOME_CASE=outside-path \
//   npm run test:ui -- --spec ./tests/agents-outside-path.e2e.mjs
const evidence=process.env.KINANA_UI_EVIDENCE||fileURLToPath(new URL('../../docs/evidence/phase-6/batch-4-5/',import.meta.url));
describe('Classifier installed outside the app PATH',()=>{
 before(()=>assert.equal(process.env.KINANA_UI_AGENT_HOME_CASE,'outside-path','run with KINANA_UI_AGENT_HOME_CASE=outside-path'));
 it('finds Claude in ~/.local/bin and classifies through it',async()=>{
  const bootstrap=await browser.tauri.execute(async({core})=>core.invoke('bootstrap'));assert.ok(bootstrap.agents.find(a=>a.name==='claude').available,JSON.stringify(bootstrap.agents));
  await $('[data-testid="nav-settings"]').click();await $('.agent-settings').waitForDisplayed();await browser.saveScreenshot(path.join(evidence,'agent-outside-path-ar.png'));
  await $('[data-testid="nav-vault"]').click();await $('[data-testid="skill-emil-color"] .skill-select').click();await $('[data-testid="detail-classify"]').waitForEnabled();await $('[data-testid="detail-classify"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();await $('[data-testid="confirm-classify"]').click();await $('[data-testid="classification-result"]').waitForDisplayed({timeout:60000});
  const detail=await browser.tauri.execute(async({core})=>core.invoke('skill_detail',{id:'emil-color'}));assert.equal(detail.detail.metadata.classification.classified_by,'claude',await $('[data-testid="classification-result"]').getText());assert.ok(detail.detail.metadata.proposal);
  const agentCase=JSON.parse(readFileSync(path.join(evidence,'agent-home-outside-path.json'),'utf8'));const calls=readFileSync(agentCase.trace,'utf8').trim().split('\n').length;assert.equal(calls,1);
  writeFileSync(path.join(evidence,'agent-outside-path.json'),JSON.stringify({restricted_path:true,agent_home:'outside-path',fake_claude_calls:calls,classified_by:'claude'},null,2)+'\n');
 });
});
