import {syntheticSkill} from './support/synthetic-skill.mjs';
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdirSync,existsSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=fileURLToPath(new URL('../../',import.meta.url)),evidence=process.env.KINANA_UI_EVIDENCE||path.join(root,'docs/evidence/phase-6/batch-4-5');
const home=process.env.KINANA_UI_HOME,base=process.env.KINANA_UI_BASE;
async function invoke(command,params={}){return browser.tauri.execute(async({core},command,params)=>core.invoke(command,params),command,params);}
let classified,agentCalls=0,vault,skill;
describe('First launch on an empty HOME — 6.4',()=>{
 it('creates a vault, scans a synthetic installed skill, imports and shows the classification cost',async()=>{
  await $('[data-testid="onboarding"]').waitForDisplayed();assert.equal(existsSync(path.join(home,'SkillsBox')),false);const sourceProject=path.join(base,'مشروع البداية');skill=path.join(sourceProject,'.claude/skills/emil-performance');mkdirSync(path.dirname(skill),{recursive:true});syntheticSkill(skill,'emil-performance');
  vault=path.join(home,'مخزن البداية');await $('[data-testid="settings-vault-path"]').setValue(vault);await $('[data-testid="settings-initialize"]').click();await $('[data-testid="settings-preview"]').waitForDisplayed();assert.equal(existsSync(vault),false);await browser.saveScreenshot(path.join(evidence,'onboarding-create-preview.png'));await $('[data-testid="confirm-settings"]').click();await browser.waitUntil(async()=>!await $('.settings-confirm-dialog').isExisting());await $('[data-testid="onboarding-next"]').waitForDisplayed();await $('button=السابق').click();
  await $('[data-testid="settings-select-vault"]').click();
  await $('[data-testid="settings-preview"]').waitForDisplayed();
  const next = await $('[data-testid="confirm-settings"]');
  assert.equal(await next.getText(),'التالي');
  await next.click();
  await browser.waitUntil(async()=>!await $('.settings-confirm-dialog').isExisting());
  await browser.saveScreenshot(path.join(evidence,'onboarding-agents-ar.png'));await $('[data-testid="onboarding-next"]').click();await $('[data-testid="manual-scan-toggle"]').click();await $('[data-testid="settings-scan-paths"]').setValue(sourceProject);await $('[data-testid="settings-scan"]').click();await $('[data-testid="scan-results"]').waitForDisplayed();if(await $('[data-testid="scan-allow-risky"]').isExisting())await $('[data-testid="scan-allow-risky"]').click();await $('[data-testid="confirm-scan-import"]').waitForEnabled();await $('[data-testid="confirm-scan-import"]').click();await browser.waitUntil(async()=>!await $('.scan-dialog').isExisting());await $('[data-testid="onboarding-classify-skill"]').click();await $('[role="option"]').click();await $('[data-testid="onboarding-classify"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();const before=await invoke('skill_detail',{id:'emil-performance'});assert.equal(before.detail.metadata.description.active,'original');await browser.saveScreenshot(path.join(evidence,'onboarding-classify-estimate.png'));
 });
 it('classifies through the UI with the isolated provider fixture',async function(){
  this.timeout(180000);
  await $('[data-testid="confirm-classify"]').click();await $('[data-testid="classification-result"]').waitForDisplayed({timeout:150000});const after=await invoke('skill_detail',{id:'emil-performance'});assert.equal(after.detail.metadata.classification.classified_by,'claude',await $('[data-testid="classification-result"]').getText());// r5 (owner, 2026-11-24): on by default, a clean skill's proposed description is approved automatically after the run, in its own commit, and the window says so.
  assert.equal(after.detail.metadata.proposal,undefined);assert.equal(after.detail.metadata.description.active,'improved');assert.ok(after.detail.metadata.description.approved_at);
  await $('[data-testid="auto-approval"]').waitForDisplayed();assert.equal(await $('[data-testid="auto-approval-count"]').getAttribute('data-count'),'1');classified='claude';
  const trace=readFileSync(path.join(base,'classification-trace.jsonl'),'utf8').trim().split('\n').map(line=>JSON.parse(line));
  assert.equal(trace.length,1);
  assert.equal(trace[0].fixture,true);
  assert.match(await $('[data-testid="classification-agent-calls"]').getText(),/[1١]/);
  agentCalls=trace.length;
 });
 it('finishes onboarding',async()=>{
  await $('[data-testid="cancel-classify"]').click();await $('[data-testid="onboarding-next"]').click();await $('[data-testid="onboarding-router"]').click();
  await $('[data-testid="settings-preview"]').waitForDisplayed();
  await $('[data-testid="confirm-settings"]').click();
  await browser.waitUntil(async()=>!await $('.settings-confirm-dialog').isExisting());
  for(const agent of ['.claude','.agents'])assert.ok(existsSync(path.join(home,agent,'skills/kinana/.skillsbox-router.toml')));
  await $('[data-testid="onboarding-finish"]').click();await $('[data-testid="skill-emil-performance"]').waitForDisplayed();await browser.saveScreenshot(path.join(evidence,'onboarding-finished-ar.png'));writeFileSync(path.join(evidence,'onboarding-result.json'),JSON.stringify({agent_calls:agentCalls,fresh_home:home,vault,source:skill,classified_by:classified,proposal_created:classified==='claude',active:'original',live_agent:false,preferences:readFileSync(path.join(home,'.kinana/desktop.toml'),'utf8'),no_terminal_in_user_flow:true},null,2)+'\n');
 });
});
