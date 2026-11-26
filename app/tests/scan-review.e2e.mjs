import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,renameSync,existsSync,readdirSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
// Round 16: a first-run scan with new, duplicate, drifted (two versions of one name) and invalid skills.
const state=JSON.parse(readFileSync(process.env.KINANA_SCAN_UI_STATE,'utf8'));
const evidence=path.dirname(process.env.KINANA_SCAN_UI_STATE);
const {vault,skills,home,base}=state;
function cli(args){const r=spawnSync(state.cli,['--vault',vault,'--json',...args],{env:{...process.env,HOME:home},encoding:'utf8'});assert.equal(r.status,0,r.stdout+r.stderr);return JSON.parse(r.stdout);}
const head=()=>spawnSync('git',['-C',vault,'rev-parse','HEAD'],{encoding:'utf8'}).stdout.trim();
// Counts may use Arabic-Indic digits.
const n=value=>`(?:${value}|${new Intl.NumberFormat('ar').format(value)})`;
const checked=async id=>(await $(`[data-testid="${id}"]`).getAttribute('data-state'))==='checked';
async function scan(){await $('[data-testid="settings-scan"]').click();await $('[data-testid="scan-results"]').waitForDisplayed({timeout:60000});await $('[data-testid="scan-actions"]').waitForDisplayed();await browser.waitUntil(async()=>!await $('[data-testid="scan-progress"]').isExisting(),{timeout:60000});}
async function inView(selector){return browser.execute(s=>{const r=document.querySelector(s).getBoundingClientRect();return r.top>=0&&r.bottom<=window.innerHeight&&r.height>0;},selector);}
const conflict='emil-design-foundations';
let initialHead;
describe('Scan review — first run with duplicates and drifted versions (round 16)',()=>{
 before(async()=>{await browser.setWindowSize(1100,760);initialHead=head();});
 it('selects only new skills, asks for one primary version per name, and folds audit notes',async()=>{
  await $('[data-testid="nav-settings"]').click();await $('[data-testid="settings-tab-sources"]').click();
  await $('[data-testid="manual-scan-toggle"]').click();await $('[data-testid="settings-scan-paths"]').setValue(path.dirname(path.dirname(skills)));await scan();
  const counts=await $('[data-testid="scan-counts"]').getText();
  for(const [label,value] of [['جديدة',14],['مكررة',1],['منحرفة',1],['غير صالحة',1]])assert.match(counts,new RegExp(`${label}: ${n(value)}`),counts);
  const boxes=await $$('[data-testid="scan-new"] [data-testid^="scan-select-"]');assert.equal(boxes.length,13);
  for(const box of boxes)assert.equal(await box.getAttribute('data-state'),'checked');
  assert.equal(await $(`[data-testid="scan-select-${conflict}"]`).isExisting(),false);
  assert.equal(await $(`[data-testid="scan-primary-${conflict}-none"]`).isSelected(),true);
  assert.equal(await $(`[data-testid="scan-primary-${conflict}-0"]`).isSelected(),false);
  assert.equal(await $(`[data-testid="scan-primary-${conflict}-1"]`).isSelected(),false);
  assert.match(await $('[data-testid="scan-duplicates"] summary').getText(),new RegExp(`مكررة.*${n(1)}`));
  assert.match(await $('[data-testid="scan-broken"] summary').getText(),new RegExp(n(1)));
  assert.match(await $('[data-testid="confirm-scan-import"]').getText(),new RegExp(`\\(${n(13)}\\)`));
  // Notes are folded: a count in the summary, the notes only after opening.
  const folds=await $$('[data-testid="scan-new"] details.scan-audit');assert.ok(folds.length>0,'fixture has audit notes');
  const fold=folds[0];assert.equal(await fold.getAttribute('open'),null);assert.equal((await fold.$$('.audit-details')).length,0);assert.match(await fold.$('summary').getText(),/ملاحظات الفحص: /);
  await fold.$('summary').click();await fold.$('.audit-details').waitForDisplayed();await fold.$('summary').click();
  await browser.saveScreenshot(path.join(evidence,'scan-review-ar.png'));
  assert.equal(head(),initialHead);
 });
 it('shows an engine error beside the buttons and keeps the list and choices',async()=>{
  await $('[data-testid="scan-select-emil-animations"]').click();assert.equal(await checked('scan-select-emil-animations'),false);
  const doomed=path.join(skills,'emil-marketing-pages'),aside=path.join(base,'aside-marketing-pages');renameSync(doomed,aside);
  try{
   if(await $('[data-testid="scan-allow-risky"]').isExisting()&&!await checked('scan-allow-risky'))await $('[data-testid="scan-allow-risky"]').click();
   await $('[data-testid="confirm-scan-import"]').waitForEnabled();await $('[data-testid="confirm-scan-import"]').click();await $('[data-testid="scan-error"]').waitForDisplayed({timeout:30000});
   assert.match(await $('[data-testid="scan-error"]').getText(),/IMPORT_PREVIEW_CHANGED.*أعد المعاينة/);assert.ok(await inView('[data-testid="scan-error"]'),'error is in view');assert.ok(await inView('[data-testid="confirm-scan-import"]'));
   await $('[data-testid="confirm-scan-import"]').waitForEnabled();assert.equal(await $('[data-testid="scan-results"]').isDisplayed(),true);assert.equal(await checked('scan-select-emil-animations'),false);
   await browser.saveScreenshot(path.join(evidence,'scan-error-in-view-ar.png'));
  }finally{renameSync(aside,doomed);}
  assert.equal(head(),initialHead);assert.deepEqual(cli(['list']),[]);
 });
 it('counts each skill while adding, and a cancel returns to the same list with nothing saved',async()=>{
  await $(`[data-testid="scan-primary-${conflict}-1"]`).click();assert.match(await $('[data-testid="confirm-scan-import"]').getText(),new RegExp(`\\(${n(13)}\\)`));
  await $('[data-testid="confirm-scan-import"]').waitForEnabled();await $('[data-testid="confirm-scan-import"]').click();
  const count=$('[data-testid="scan-progress-count"]');await count.waitForDisplayed({timeout:30000});await browser.waitUntil(async()=>/[1-9١-٩]/.test((await count.getText()).split(/من|of/)[0]),{timeout:30000});
  const progress=await count.getText();assert.match(progress,new RegExp(`من ${n(13)}`));await browser.saveScreenshot(path.join(evidence,'scan-progress-ar.png'));
  await $('[data-testid="cancel-scan-import"]').click();await $('[data-testid="scan-cancelled"]').waitForDisplayed({timeout:30000});
  assert.equal(head(),initialHead);assert.deepEqual(cli(['list']),[]);assert.deepEqual(readdirSync(path.join(vault,'skills')),[]);
  assert.equal(await $(`[data-testid="scan-primary-${conflict}-1"]`).isSelected(),true);assert.equal(await checked('scan-select-emil-animations'),false);assert.equal(await $('[data-testid="scan-results"]').isDisplayed(),true);
  writeFileSync(path.join(evidence,'cancel.json'),JSON.stringify({progress_seen:progress,head_unchanged:true,vault_skills:[]},null,2)+'\n');
 });
 it('closing and scanning the same folders again returns to the same list',async()=>{
  await $('[data-testid="cancel-scan"]').click();await browser.waitUntil(async()=>!await $('.scan-dialog').isExisting());
  await scan();assert.equal(await $(`[data-testid="scan-primary-${conflict}-1"]`).isSelected(),true);assert.equal(await checked('scan-select-emil-animations'),false);
 });
 it('adds each new skill once and records duplicates and the other version as locations',async()=>{
  await $('[data-testid="confirm-scan-import"]').waitForEnabled();await $('[data-testid="confirm-scan-import"]').click();await browser.waitUntil(async()=>!await $('.scan-dialog').isExisting(),{timeout:90000});
  const ids=cli(['list']).map(s=>s.id).sort();
  const expected=[...state.fresh.filter(id=>id!=='emil-animations'),'emil-color',conflict].sort();assert.deepEqual(ids,expected);
  assert.deepEqual(readdirSync(path.join(vault,'skills')).sort(),expected);
  const color=cli(['show','emil-color']);for(const folder of ['color','color-copy'])assert.ok(color.locations.includes(path.join(skills,folder)),folder);
  const design=cli(['show',conflict]);assert.equal(readFileSync(path.join(vault,'skills',conflict,'SKILL.md'),'utf8'),readFileSync(path.join(skills,'design-b/SKILL.md'),'utf8'));
  assert.deepEqual(design.variants.map(v=>v.path),[path.join(skills,'design-a')]);
  assert.equal(existsSync(path.join(vault,'skills','broken')),false);
  const doctor=spawnSync(state.cli,['--vault',vault,'--json','doctor'],{env:{...process.env,HOME:home},encoding:'utf8'});assert.equal(doctor.status,0,doctor.stdout);
  writeFileSync(path.join(evidence,'result.json'),JSON.stringify({imported:ids,color_locations:color.locations,design_variants:design.variants,doctor_healthy:JSON.parse(doctor.stdout).healthy},null,2)+'\n');
 });
});
