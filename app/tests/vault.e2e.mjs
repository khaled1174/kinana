import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, readdirSync, lstatSync, mkdirSync, existsSync, cpSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import http from 'node:http';
const root=fileURLToPath(new URL('../../',import.meta.url));const evidence=process.env.KINANA_UI_EVIDENCE||path.join(root,'docs/evidence/phase-6/batch-4-5');
const home=process.env.KINANA_UI_HOME;const base=process.env.KINANA_UI_BASE;const vault=path.join(home,'SkillsBox');
function cli(args){const r=spawnSync(process.env.KINANA_TEST_CLI,['--vault',vault,'--json',...args],{env:{...process.env,HOME:home},encoding:'utf8'});assert.equal(r.status,0,r.stdout+r.stderr);return JSON.parse(r.stdout);}
function tree(dir){return readdirSync(dir).sort().flatMap(name=>{const p=path.join(dir,name);return lstatSync(p).isDirectory()?[[name,tree(p)]]:[[name,createHash('sha256').update(readFileSync(p)).digest('hex')]];});}
async function invoke(cmd,args={}) {return browser.tauri.execute(async ({core},command,params)=>core.invoke(command,params),cmd,args);}
async function chooseOption(testId,label){await $(`[data-testid="${testId}"]`).click();const choices=await $$('[role="option"]');for(const choice of choices){if((await choice.getText())===label){await choice.click();return;}}throw new Error(`Missing option: ${label}`);}
async function waitCount(expected){await browser.waitUntil(async()=>{const report=await invoke('catalog',{filter:{}});return report.total===expected;});}
async function undoUI(){await $('[data-testid="undo"]').waitForEnabled();await $('[data-testid="undo"]').click();await $('[data-testid="confirm-undo"]').waitForDisplayed();await $('[data-testid="confirm-undo"]').click();}
async function openImport(){await $('[data-testid="add-skill"]').waitForEnabled();await $('[data-testid="add-skill"]').click();await $('[data-testid="import-source"]').waitForDisplayed();}
describe('Real file-backed vault — batches 6.4–6.5',()=>{
 it('shows seven real skills in both languages, and the complete word comparison',async()=>{
  await $('[data-testid="skill-emil-design-foundations"]').waitForDisplayed();
  const before={skills:tree(path.join(vault,'skills')),dist:tree(path.join(vault,'dist'))};
  const detail=cli(['show','emil-design-foundations']);assert.equal(detail.metadata.classification.classified_by,'claude');assert.ok(detail.metadata.proposal);assert.equal(detail.metadata.description.active,'original');
  await $('[data-testid="skill-emil-design-foundations"] .skill-select').click();await $('[data-testid="pending-proposal"]').waitForDisplayed();
  assert.equal(await $$('.description-comparison del, .description-comparison ins').length,0);
  await $('[data-testid="toggle-description-diff"]').click();
  const comparison=await browser.execute(()=>{const original=document.querySelector('[data-testid="original-description"]');const proposed=document.querySelector('[data-testid="proposed-description"]');return{original:original.textContent,proposed:proposed.textContent,removed:original.querySelectorAll('del').length,added:proposed.querySelectorAll('ins').length,folds:document.querySelectorAll('.description-comparison details').length,clamp:getComputedStyle(original).webkitLineClamp,order:!!(original.compareDocumentPosition(proposed)&Node.DOCUMENT_POSITION_FOLLOWING)};});
  assert.equal(comparison.original,detail.metadata.description.original);assert.equal(comparison.proposed,detail.metadata.proposal.improved_description);assert.ok(comparison.added>0&&comparison.removed>0);assert.equal(comparison.folds,0);assert.equal(comparison.clamp,'none');assert.equal(comparison.order,true);
  await browser.setWindowSize(1400,1050);await browser.waitUntil(async()=>await browser.execute(()=>{const r=document.querySelector('[data-testid="pending-proposal"]').getBoundingClientRect();return r.bottom<=innerHeight;}));await browser.execute(()=>document.activeElement?.blur());await browser.saveScreenshot(path.join(evidence,'description-review.png'));
  await browser.setWindowSize(1100,760);await $('[data-testid="skill-emil-color"] .skill-select').click();await $('[data-testid="detail-approximate"]').waitForDisplayed();await $('[data-testid="safe-preview"]').waitForDisplayed();
  const density=[];
  for(const locale of ['ar','en']){
   const geometry=await browser.execute(()=>{const list=document.querySelector('.skill-list').getBoundingClientRect();return{locale:document.documentElement.lang,width:innerWidth,height:innerHeight,visible:Array.from(document.querySelectorAll('.skill-row')).filter(e=>{const r=e.getBoundingClientRect();return r.top>=list.top&&r.bottom<=Math.min(list.bottom,innerHeight);}).map(e=>e.dataset.testid),pathVisible:!!document.querySelector('.vault-path'),folderTooltip:document.querySelector('[data-testid="choose-vault"]').title,filtersVisible:!!document.querySelector('[data-testid="filter-category"]')};});
   geometry.window=await browser.getWindowSize();assert.equal(geometry.window.width,1100);assert.equal(geometry.window.height,760);assert.equal(geometry.locale,locale);assert.equal(geometry.width,1100);assert.ok(geometry.height<=760);assert.ok(geometry.visible.length>=7,JSON.stringify(geometry));assert.equal(geometry.pathVisible,false);assert.equal(geometry.folderTooltip,vault);assert.equal(geometry.filtersVisible,false);density.push(geometry);
   await browser.execute(()=>document.activeElement?.blur());await browser.saveScreenshot(path.join(evidence,`vault-${locale}.png`));await $('[data-testid="language"]').click();
  }
  writeFileSync(path.join(evidence,'layout-and-review.json'),JSON.stringify({density,comparison,source_and_dist_unchanged:true},null,2)+'\n');
  assert.deepEqual({skills:tree(path.join(vault,'skills')),dist:tree(path.join(vault,'dist'))},before);
  writeFileSync(path.join(evidence,'preview-readonly.json'),JSON.stringify({before,after:before,source_files_changed:false},null,2)+'\n');
 });
 it('returns the same search and combined filters as the CLI',async()=>{
  await $('[data-testid="search"]').setValue('Typography');
  await browser.waitUntil(async()=>await $$('.skill-list [data-testid^="skill-"]').length===cli(['list','--query','Typography']).length);
  assert.equal(await $('[data-testid="skill-emil-typography"]').isDisplayed(),true);
  await $('[data-testid="search"]').setValue('zzzzmissingresult');
  await browser.waitUntil(async()=>(await $('[data-testid="result-count"]').getAttribute('data-count'))==='0');
  await $('[data-testid="search"]').setValue('');
  await $('[data-testid="toggle-filters"]').click();await chooseOption('filter-state','وصف ينتظر المراجعة');
  await browser.waitUntil(async()=>(await $('[data-testid="result-count"]').getAttribute('data-count'))==='1');
  const actual=await invoke('catalog',{filter:{state:'proposal'}});assert.deepEqual(actual.items.map(i=>i.skill),cli(['list','--state','proposal']));
  await $('[data-testid="toggle-filters"]').click();assert.equal(await $('[data-filter="state"]').isDisplayed(),true);await $('[data-filter="state"]').click();
 });
 it('explains audit levels and estimates classification before any model call or write',async()=>{
  const levels=path.join(base,'audit-levels');const clean=path.join(levels,'clean');mkdirSync(clean,{recursive:true});writeFileSync(path.join(clean,'SKILL.md'),'---\nname: audit-clean\ndescription: Read a local reference\n---\nA local reference.\n');cpSync(path.join(root,'tests/fixtures/audit/script-file'),path.join(levels,'script'),{recursive:true});cpSync(path.join(root,'tests/fixtures/audit/destructive-command'),path.join(levels,'destructive'),{recursive:true});cli(['add',levels,'--all','--allow-risky']);
  await browser.refresh();await $('[data-testid="skill-audit-clean"]').waitForDisplayed();
  const audit=await browser.execute(()=>Array.from(document.querySelectorAll('.skill-row [data-audit]')).map(e=>({level:e.dataset.audit,label:e.getAttribute('aria-label'),title:e.title,dot:!!e.querySelector('.audit-dot'),color:e.querySelector('.audit-dot')?getComputedStyle(e.querySelector('.audit-dot')).backgroundColor:null})));
  assert.ok(audit.some(a=>a.level==='clean'));assert.ok(audit.some(a=>a.level==='review'));assert.ok(audit.some(a=>a.level==='risky'));
  for(const a of audit){assert.equal(a.label,a.title);assert.ok(a.title.length>10);if(a.level==='clean')assert.equal(a.dot,false);if(a.level==='review')assert.notEqual(a.color,audit.find(i=>i.level==='risky').color);}writeFileSync(path.join(evidence,'audit-indicators.json'),JSON.stringify(audit,null,2)+'\n');cli(['undo']);await browser.refresh();await browser.waitUntil(async()=>await $('[data-testid="result-count"]').getAttribute('data-count')==='7');await $('[data-testid="classify-emil-color"]').waitForEnabled();
  const before=tree(vault);await $('[data-testid="skill-emil-color"] .skill-select').click();await $('[data-testid="detail-classify"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();
  const preview=await invoke('preview_classify',{id:'emil-color'});const expected=cli(['classify','emil-color','--agent','claude','--dry-run']);assert.deepEqual(preview.estimated_tokens,expected.estimated_tokens);assert.equal(preview.agent_calls,0);
  assert.deepEqual(tree(vault),before);await $('[data-testid="cancel-classify"]').click();assert.deepEqual(tree(vault),before);
  await invoke('start_classify',{id:'emil-color',estimate:preview.estimated_tokens,confirmed:false});await browser.waitUntil(async()=>(await invoke('classify_status')).done);const denied=await invoke('classify_status');assert.equal(denied.error.code,'CLASSIFY_CONFIRMATION_REQUIRED');assert.equal(denied.report,null);await invoke('cancel_classify');assert.deepEqual(tree(vault),before);
  await invoke('start_classify',{id:'emil-color',estimate:{...preview.estimated_tokens,total_input_tokens:preview.estimated_tokens.total_input_tokens+1},confirmed:true});await browser.waitUntil(async()=>(await invoke('classify_status')).done);const changed=await invoke('classify_status');assert.equal(changed.error.code,'CLASSIFY_ESTIMATE_CHANGED');assert.equal(changed.report,null);await invoke('cancel_classify');assert.deepEqual(tree(vault),before);
  writeFileSync(path.join(evidence,'classify-cost-preview.json'),JSON.stringify({preview,cli_estimate:expected.estimated_tokens,confirmation_rejected:denied,stale_estimate:changed,no_writes:true},null,2)+'\n');
 });
 it('pins and unpins through the UI without changing source or dist',async()=>{
  const before={skills:tree(path.join(vault,'skills')),dist:tree(path.join(vault,'dist'))};
  await $('[data-testid="pin-emil-typography"]').click();
  await browser.waitUntil(async()=>cli(['list'])[0].id==='emil-typography'&&cli(['list'])[0].pinned);
  await $('[data-testid="filter-pinned"]').click();
  await browser.waitUntil(async()=>(await $('[data-testid="result-count"]').getAttribute('data-count'))==='1');
  await $('[data-testid="filter-pinned"]').click();
  await browser.waitUntil(async()=>await $('[data-testid="result-count"]').getAttribute('data-count')==='7');await $('[data-testid="pin-emil-typography"]').waitForEnabled();
  await $('[data-testid="pin-emil-typography"]').click();
  await browser.waitUntil(async()=>!cli(['show','emil-typography']).skill.pinned);
  await browser.waitUntil(async()=>await $('.skill-row').getAttribute('data-testid')==='skill-emil-color');
  await $('[data-testid="skill-emil-design-foundations"] .skill-select').waitForEnabled();
  assert.deepEqual({skills:tree(path.join(vault,'skills')),dist:tree(path.join(vault,'dist'))},before);
 });
 it('accepts the proposal, and undo restores the pending description',async()=>{
  await $('[data-testid="skill-emil-design-foundations"] .skill-select').waitForEnabled();
  await $('[data-testid="skill-emil-design-foundations"] .skill-select').click();
  await $('[data-testid="accept-description"]').waitForDisplayed();
  const before=cli(['show','emil-design-foundations']);const source=tree(path.join(vault,'skills'));
  await $('[data-testid="accept-description"]').scrollIntoView();await $('[data-testid="accept-description"]').click();await $('[data-testid="confirm-review"]').click();
  await browser.waitUntil(async()=>cli(['show','emil-design-foundations']).metadata.description.active==='improved');
  assert.match(readFileSync(path.join(vault,'dist/emil-design-foundations/SKILL.md'),'utf8'),/./);
  await undoUI();
  await browser.waitUntil(async()=>Boolean(cli(['show','emil-design-foundations']).metadata.proposal));
  assert.equal(cli(['show','emil-design-foundations']).metadata.description.active,before.metadata.description.active);
  assert.deepEqual(tree(path.join(vault,'skills')),source);await $('[data-testid="pending-proposal"]').waitForDisplayed();await browser.waitUntil(async()=>await $('[data-testid="skill-details"]').getAttribute('data-active')==='original');
 });
 it('changes category without approving the description, and can reject then undo the proposal',async()=>{
  await $('[data-testid="tab-info"]').scrollIntoView();await $('[data-testid="tab-info"]').click();await $('[data-testid="detail-category"]').waitForDisplayed();await chooseOption('detail-category','الأمان');await $('[data-testid="save-category"]').click();
  await browser.waitUntil(async()=>cli(['show','emil-design-foundations']).skill.category==='security');await browser.waitUntil(async()=>await $('[data-testid="skill-details"]').getAttribute('data-category')==='security');assert.ok(cli(['show','emil-design-foundations']).metadata.proposal);
  await undoUI();await browser.waitUntil(async()=>cli(['show','emil-design-foundations']).skill.category==='design-ui');await browser.waitUntil(async()=>await $('[data-testid="skill-details"]').getAttribute('data-category')==='design-ui');
  await $('[data-testid="reject-description"]').waitForEnabled();await $('[data-testid="reject-description"]').scrollIntoView();await $('[data-testid="reject-description"]').click();await browser.waitUntil(async()=>!cli(['show','emil-design-foundations']).metadata.proposal);
  await undoUI();await browser.waitUntil(async()=>!!cli(['show','emil-design-foundations']).metadata.proposal);
 });
 it('cancels a prepared local import without modifying the vault or source',async()=>{
  const before=tree(vault);const original=tree(path.join(base,'sources','emil-color'));
  await openImport();await $('[data-testid="import-source"]').setValue(path.join(base,'sources','emil-color'));
  await $('[data-testid="import-inspect"]').click();await browser.waitUntil(async()=>await $('.import-items').isDisplayed());
  await $('[data-testid="import-inspect"]').click();await $('[data-testid="import-confirm"]').waitForDisplayed();
  const cancel=await $('.import-dialog [data-slot=dialog-footer] button:first-child');await cancel.click();await browser.waitUntil(async()=>!(await $('.import-dialog').isExisting()));
  assert.deepEqual(tree(vault),before);assert.deepEqual(tree(path.join(base,'sources','emil-color')),original);
 });
 it('previews ZIP preparation without writes, then cancels and removes its owned scratch source',async()=>{
  const archive=path.join(base,'real-color.zip');const r=spawnSync('/usr/bin/zip',['-q',archive,'SKILL.md'],{cwd:path.join(base,'sources','emil-color'),encoding:'utf8'});assert.equal(r.status,0,r.stderr);
  const before=tree(vault);const archiveHash=createHash('sha256').update(readFileSync(archive)).digest('hex');
  const imports=path.join(home,'.kinana/imports');const owned=()=>existsSync(imports)?readdirSync(imports).filter(n=>n.startsWith('source-')):[];const previous=owned();
  await openImport();await $('[data-testid="import-source"]').setValue(archive);await $('[data-testid="import-inspect"]').click();
  await browser.waitUntil(async()=>await $('[data-testid="import-inspect"]').getText()==='تحضير المصدر ومعاينته');assert.deepEqual(owned(),previous);assert.deepEqual(tree(vault),before);
  await $('[data-testid="import-inspect"]').click();await $('[data-testid="import-confirm"]').waitForDisplayed();assert.ok(owned().length>previous.length);
  await $('.import-dialog [data-slot=dialog-footer] button:first-child').click();await browser.waitUntil(async()=>!await $('.import-dialog').isExisting());assert.deepEqual(owned(),previous);assert.deepEqual(tree(vault),before);assert.equal(createHash('sha256').update(readFileSync(archive)).digest('hex'),archiveHash);
 });
 it('renders raw text exactly, and exposes source/audit/locations through the detail tab',async()=>{
  await $('[data-testid="tab-raw"]').scrollIntoView();await $('[data-testid="tab-raw"]').click();
  assert.equal(await browser.execute(()=>document.querySelector('[data-testid="raw-skill"]').textContent),readFileSync(path.join(vault,'skills/emil-design-foundations/SKILL.md'),'utf8'));
  await $('[data-testid="tab-info"]').scrollIntoView();await $('[data-testid="tab-info"]').click();
  assert.ok((await $('.metadata-sections').getText()).includes('emil-design-foundations'));
  await $('[data-testid="tab-preview"]').click();
 });
 it('creates a bundle only after its preview and explicit confirmation',async()=>{
  await $('[data-testid="vault-tab-bundles"]').click();await $('[data-testid="new-bundle"]').click();
  await $('[data-testid="bundle-id"]').setValue('ui-reading');await $('[data-testid="bundle-title"]').setValue('واجهة · UI reading');
  await $('[role="checkbox"][aria-label="emil-design-foundations"]').click();
  await $('[data-testid="bundle-save"]').click();await browser.waitUntil(async()=>await $('[data-testid="bundle-save"]').getText()==='تأكيد الحفظ');assert.equal(cli(['bundle','list']).length,0);
  await $('[data-testid="bundle-save"]').click();await browser.waitUntil(async()=>cli(['bundle','list']).length===1);
  await $('.bundle-card button').click();await $('[data-testid="bundle-title"]').setValue('واجهة · Updated reading');await $('[data-testid="bundle-save"]').click();await browser.waitUntil(async()=>await $('[data-testid="bundle-save"]').getText()==='تأكيد الحفظ');await $('[data-testid="bundle-save"]').click();await browser.waitUntil(async()=>cli(['bundle','list'])[0].title==='واجهة · Updated reading');await browser.waitUntil(async()=>!await $('.bundle-dialog').isExisting());
  await $('[data-testid="vault-tab-groups"]').click();assert.ok(await $('.groups-tab').isDisplayed());
  await $('[data-testid="vault-tab-skills"]').click();
 });
 it('rejects every direct opener command and path-shaped skill identifier',async()=>{
  const denied=[];
  for(const command of ['plugin:opener|open_path','plugin:opener|open_url','plugin:opener|reveal_item_in_dir']) {
   const result=await browser.tauri.execute(async({core},cmd)=>{try{await core.invoke(cmd,{path:'/tmp/never.command',url:'https://example.invalid'});return {allowed:true};}catch(error){return {allowed:false,error:String(error)};}},command);
   assert.equal(result.allowed,false,command);assert.match(result.error,/not allowed by ACL/i);denied.push({command,...result});
  }
  writeFileSync(path.join(evidence,'opener-denied.json'),JSON.stringify(denied,null,2)+'\n');
  for(const id of ['../outside','/tmp/never.command','https://example.invalid','emil-design-foundations/SKILL.md']) {
   const result=await browser.tauri.execute(async({core},id)=>{try{await core.invoke('reveal_skill_folder',{id});return true;}catch{return false;}},id);assert.equal(result,false,id);
  }
  const permissions=JSON.parse(readFileSync(path.join(root,'app/src-tauri/capabilities/main.json'),'utf8')).permissions;
  assert.ok(!permissions.some(p=>String(p).startsWith('opener:')));
 });
 it('classifies a keyword skill with the fake provider only after the displayed cost is confirmed',async function(){this.timeout(180000);
  const status=spawnSync('/usr/bin/git',['-C',vault,'-c','core.hooksPath=/dev/null','status','--porcelain=v1'],{env:{...process.env,HOME:home},encoding:'utf8'});writeFileSync(path.join(evidence,'before-ui-classification-git-status.txt'),status.stdout+status.stderr);
  await $('[data-testid="skill-emil-ui-review"] .skill-select').click();await $('[data-testid="detail-classify"]').waitForDisplayed();await $('[data-testid="detail-classify"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();
  assert.equal(cli(['show','emil-ui-review']).metadata.classification.classified_by,'keywords');const source=tree(path.join(vault,'skills'));const dist=tree(path.join(vault,'dist'));
  await $('[data-testid="confirm-classify"]').click();await $('[data-testid="classification-result"]').waitForDisplayed({timeout:150000});
  const result=cli(['show','emil-ui-review']);const text=await $('[data-testid="classification-result"]').getText();const trace=readFileSync(path.join(base,'classification-trace.jsonl'),'utf8').trim().split('\n').map(JSON.parse);assert.equal(trace.length,1);assert.equal(trace[0].fixture,true);assert.match(await $('[data-testid="classification-agent-calls"]').getText(),/[1١]/);const calls=trace.length;writeFileSync(path.join(evidence,'fake-ui-classification.json'),JSON.stringify({agent_calls:calls,classified_by:result.metadata.classification.classified_by,proposal_created:!!result.metadata.proposal,active:result.metadata.description.active,visible_result:text,source_unchanged:JSON.stringify(source)===JSON.stringify(tree(path.join(vault,'skills'))),dist_unchanged:JSON.stringify(dist)===JSON.stringify(tree(path.join(vault,'dist')))},null,2)+'\n');
  assert.equal(result.metadata.classification.classified_by,'claude',text);assert.ok(result.metadata.proposal);assert.equal(result.metadata.description.active,'original');assert.deepEqual(tree(path.join(vault,'skills')),source);assert.deepEqual(tree(path.join(vault,'dist')),dist);
  await $('[data-testid="cancel-classify"]').click();await undoUI();await browser.waitUntil(async()=>cli(['show','emil-ui-review']).metadata.classification.classified_by==='keywords');
 });
 it('imports and safely previews an attack fixture without executing files or requesting resources',async()=>{
  const requests=[];const server=http.createServer((req,res)=>{requests.push(req.url);res.end('probe');});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const port=server.address().port;const source=path.join(base,'preview-security');mkdirSync(source,{recursive:true});
  const marker=path.join(base,'must-not-execute');const raw=`---\nname: preview-security\ndescription: Untrusted rendering acceptance fixture\n---\n# Preview security\n<script>window.__skill_attack=true;fetch('http://127.0.0.1:${port}/script')</script>\n<img src="http://127.0.0.1:${port}/image" onerror="window.__skill_attack=true">\n<iframe src="http://127.0.0.1:${port}/frame"></iframe>\n<svg onload="window.__skill_attack=true"></svg>\n![remote](http://127.0.0.1:${port}/markdown-image)\n![file](file://${marker})\n![data](data:image/svg+xml;base64,PHN2Zz4=)\n[explicit](http://127.0.0.1:${port}/link)\n[file](file://${marker})\n[data](data:text/html,unsafe)\nhttps://example.invalid/autolink\n\n| A | B |\n|---|---|\n| text | safe |\n\n\`\`\`sh\necho display-only\n\`\`\`\n`;
  writeFileSync(path.join(source,'SKILL.md'),raw.replaceAll('\n<','\n\n<').replaceAll('\n![','\n\n![').replaceAll('\n[','\n\n['));writeFileSync(path.join(source,'danger.command'),`#!/bin/sh\ntouch '${marker}'\n`,{mode:0o755});
  const total=cli(['list']).length;
  try{
   await openImport();await $('[data-testid="import-source"]').setValue(source);
   await $('[data-testid="import-inspect"]').click();await browser.waitUntil(async()=>await $('.import-items').isDisplayed());
   assert.equal(cli(['list']).length,total);
   await $('[data-testid="import-inspect"]').click();await $('[data-testid="import-confirm"]').waitForDisplayed();
   const checkbox=await $$('.import-dialog [role="checkbox"]');await checkbox[checkbox.length-1].click();
   await $('[data-testid="import-confirm"]').click();await waitCount(total+1);await browser.waitUntil(async()=>!(await $('.import-dialog').isExisting()));
   await $('[data-testid="skill-preview-security"]').waitForDisplayed();await $('[data-testid="skill-preview-security"] .skill-select').click();await $('[data-testid="safe-preview"]').waitForDisplayed();
   const payloadBefore=tree(path.join(vault,'skills/preview-security'));const distBefore=tree(path.join(vault,'dist/preview-security'));
   const actual=await browser.execute(()=>{const p=document.querySelector('[data-testid="safe-preview"]');return{elements:p.querySelectorAll('script,img,iframe,svg,a[href],input,object,embed').length,text:p.textContent,attack:window.__skill_attack===true,imageAlts:p.querySelectorAll('.image-alt').length,literalLinks:p.querySelectorAll('.literal-link').length,table:!!p.querySelector('table'),code:!!p.querySelector('pre code')};});
   assert.equal(actual.elements,0);assert.equal(actual.attack,false);assert.equal(actual.imageAlts,3);assert.equal(actual.literalLinks,3);assert.equal(actual.table,true);assert.equal(actual.code,true);assert.ok(actual.text.includes('<script>'));assert.equal(requests.length,0);
   await $('[data-testid="tab-raw"]').scrollIntoView();await $('[data-testid="tab-raw"]').click();await $('[data-testid="raw-skill"]').waitForDisplayed();assert.equal(await browser.execute(()=>document.querySelector('[data-testid="raw-skill"]').textContent),readFileSync(path.join(source,'SKILL.md'),'utf8'));
   assert.deepEqual(tree(path.join(vault,'skills/preview-security')),payloadBefore);assert.deepEqual(tree(path.join(vault,'dist/preview-security')),distBefore);
   await $('[data-testid="reveal-folder"]').click();assert.equal(existsSync(marker),false);
   writeFileSync(path.join(evidence,'safe-preview.json'),JSON.stringify({requests,dom:actual,source_unchanged:true,dist_unchanged:true,command_file_executed:existsSync(marker)},null,2)+'\n');
   await undoUI();await waitCount(total);
   await invoke('reveal_skill_folder',{id:'emil-design-foundations'});
  }finally{await new Promise(resolve=>server.close(resolve));}
 });
});
